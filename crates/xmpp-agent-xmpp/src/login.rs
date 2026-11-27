//! The connector the stream uses to (re)connect, in place of `StanzaStream::new_c2s`, for two reasons.
//!
//! 1. A wrong password is final. `new_c2s` retries it forever; here it is reported once, through `fatal`.
//! 2. Prosody answers a stream-management resumption of a session it no longer has (after a restart) with a `<failed/>` that has
//!    no `h` attribute, which xmpp-parsers 0.23 cannot parse: the stream closes and the next attempt resumes again, forever.
//!    A connection is handed out with stream management on, so a session that is still alive resumes. When the previous
//!    connection never became usable (`unconfirmed` still set), the next one hides the `sm` feature, so the stream binds anew.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sasl::common::Credentials;
use tokio::sync::{mpsc, oneshot};
use tokio_xmpp::connect::{DirectTlsServerConnector, ServerConnector};
use tokio_xmpp::jid::Jid;
use tokio_xmpp::parsers::ns;
use tokio_xmpp::parsers::stream_features::StreamFeatures;
use tokio_xmpp::stanzastream::Connection;
use tokio_xmpp::xmlstream::{StreamHeader, Timeouts, XmppStream};
use tokio_xmpp::{Error as XmppError, client_login};

const MAX_DELAY: Duration = Duration::from_secs(30);

pub type Connector = Box<dyn FnMut(Option<String>, oneshot::Sender<Connection>) + Send + 'static>;

/// Set when a connection was handed out, cleared by the owner once the stream reports a session (`Reset` or `Resumed`).
pub type Unconfirmed = Arc<AtomicBool>;

async fn login(
    server: &DirectTlsServerConnector,
    jid: &Jid,
    password: &str,
    timeouts: Timeouts,
    hide_sm: bool,
) -> Result<
    (
        StreamFeatures,
        XmppStream<<DirectTlsServerConnector as ServerConnector>::Stream>,
    ),
    XmppError,
> {
    let username = jid.node().ok_or(XmppError::InvalidState)?.as_str();
    let (stream, channel_binding) = server.connect(jid, ns::JABBER_CLIENT, timeouts).await?;
    let (features, stream) = stream.recv_features().await?;
    let creds = Credentials::default()
        .with_username(username)
        .with_password(password)
        .with_channel_binding(channel_binding);
    let stream = client_login(stream, features.sasl_mechanisms, creds).await?;
    let header = StreamHeader {
        to: Some(Cow::Borrowed(jid.domain().as_str())),
        from: None,
        id: None,
    };
    let (mut features, stream) = stream.send_header(header).await?.recv_features().await?;
    if hide_sm {
        features.stream_management = None;
    }
    Ok((features, stream))
}

pub fn connector(
    server: DirectTlsServerConnector,
    jid: Jid,
    password: String,
    timeouts: Timeouts,
    unconfirmed: Unconfirmed,
    fatal: mpsc::UnboundedSender<XmppError>,
) -> Connector {
    Box::new(move |_location, mut slot| {
        let (server, jid, password) = (server.clone(), jid.clone(), password.clone());
        let (unconfirmed, fatal) = (unconfirmed.clone(), fatal.clone());
        tokio::spawn(async move {
            let hide_sm = unconfirmed.load(Ordering::SeqCst);
            let mut delay = Duration::from_secs(1);
            loop {
                match login(&server, &jid, &password, timeouts, hide_sm).await {
                    Ok((features, stream)) => {
                        unconfirmed.store(true, Ordering::SeqCst);
                        let conn = Connection {
                            stream: stream.box_stream(),
                            features,
                            identity: jid,
                        };
                        // the stream is gone: nobody will use this connection
                        let _ = slot.send(conn);
                        return;
                    }
                    Err(e @ XmppError::Auth(_)) => {
                        let _ = fatal.send(e);
                        // Dropping the slot now would make the stream panic; hold it until the stream is closed.
                        slot.closed().await;
                        return;
                    }
                    Err(_) => {
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(MAX_DELAY);
                    }
                }
            }
        });
    })
}
