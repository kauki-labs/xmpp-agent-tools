// E16 spike: can a client on tokio-xmpp 6.0.0 and xmpp-parsers 0.23.0 do what the agent client needs, against a real Prosody?
use futures::StreamExt;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;
use tokio_xmpp::connect::DnsConfig;
use tokio_xmpp::xmlstream::Timeouts;
use tokio_xmpp::{rustls, Client, Event, Stanza};
use xmpp_parsers::iq::Iq;
use xmpp_parsers::jid::{BareJid, Jid};
use xmpp_parsers::mam;
use xmpp_parsers::message::{Message, MessageType};
use xmpp_parsers::message_correct::Replace;
use xmpp_parsers::minidom::Element;
use xmpp_parsers::muc::user::MucUser;
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::Presence;
use xmpp_parsers::reactions::{Reaction, Reactions};
use xmpp_parsers::rsm::SetQuery;

static FAILS: AtomicUsize = AtomicUsize::new(0);
const PORT: u16 = 15223;
const ROOM: &str = "spike@rooms.localhost";

fn step(name: &str, ok: bool, detail: impl std::fmt::Display) {
    if !ok {
        FAILS.fetch_add(1, Ordering::SeqCst);
    }
    println!("{} {} :: {}", if ok { "PASS" } else { "FAIL" }, name, detail);
}

async fn connect(user: &str) -> Result<Client, String> {
    let jid = BareJid::from_str(&format!("{user}@localhost")).unwrap();
    let dns = DnsConfig::NoSrv { host: "localhost".into(), port: PORT, resolver: None };
    let mut c = Client::new_direct_tls_with_config(jid, format!("pw-{user}"), dns, Timeouts::default());
    loop {
        match timeout(Duration::from_secs(10), c.next()).await {
            Ok(Some(Event::Online { bound_jid, .. })) => {
                println!("   {user} online as {bound_jid}");
                return Ok(c);
            }
            Ok(Some(Event::Disconnected(e))) => return Err(format!("disconnected: {e:?}")),
            Ok(Some(_)) => {}
            Ok(None) => return Err("stream ended".into()),
            Err(_) => return Err("timeout waiting for online".into()),
        }
    }
}

/// Read stanzas until `f` returns Some, or the timeout passes.
async fn wait<T>(c: &mut Client, secs: u64, mut f: impl FnMut(&Stanza) -> Option<T>) -> Option<T> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match timeout(left, c.next()).await {
            Ok(Some(Event::Stanza(s))) => {
                if let Some(v) = f(&s) {
                    return Some(v);
                }
            }
            Ok(Some(Event::Disconnected(e))) => {
                println!("   disconnected: {e:?}");
                return None;
            }
            Ok(Some(_)) => {}
            _ => return None,
        }
    }
}

fn muc_jids(p: &Presence) -> Vec<String> {
    let mut out = vec![];
    for el in &p.payloads {
        if let Ok(u) = MucUser::try_from(el.clone()) {
            for item in u.items {
                if let Some(j) = item.jid {
                    out.push(j.to_string());
                }
            }
        }
    }
    out
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider().install_default().expect("crypto provider");

    // 1. direct TLS with a verified certificate (the test CA comes from SSL_CERT_FILE)
    let mut alice = match connect("alice").await {
        Ok(c) => { step("1 direct TLS 5223-style, verified cert, SASL, bind", true, "online"); c }
        Err(e) => { step("1 direct TLS, verified cert", false, e); std::process::exit(1); }
    };
    let mut bob = connect("bob").await.expect("bob");

    // 2. alice joins the room and unlocks it
    let room_alice = Jid::from_str(&format!("{ROOM}/alice")).unwrap();
    let join = Presence::available().with_to(room_alice.clone()).with_payload(Muc::new());
    alice.send_stanza(join.into()).await.unwrap();
    let joined = wait(&mut alice, 10, |s| match s {
        Stanza::Presence(p) if p.from.as_ref().map(|f| f.to_string()) == Some(format!("{ROOM}/alice")) => Some(muc_jids(p)),
        _ => None,
    }).await;
    step("2a join a MUC, see own presence", joined.is_some(), format!("{joined:?}"));
    let cfg = Element::builder("query", "http://jabber.org/protocol/muc#owner")
        .append(Element::builder("x", "jabber:x:data").attr(xmpp_parsers::minidom::rxml::NcName::try_from("type").unwrap(), "submit"))
        .build();
    let iq = Iq::Set { from: None, to: Some(Jid::from_str(ROOM).unwrap()), id: "cfg1".into(), payload: cfg };
    alice.send_stanza(iq.into()).await.unwrap();
    let cfg_ok = wait(&mut alice, 10, |s| match s { Stanza::Iq(Iq::Result { id, .. }) if id == "cfg1" => Some(()), _ => None }).await;
    step("2b accept the default room configuration (unlock)", cfg_ok.is_some(), "iq result");

    // 3. bob joins; both should see real JIDs (muc_room_default_public_jids = true on this server)
    let room_bob = Jid::from_str(&format!("{ROOM}/bob")).unwrap();
    bob.send_stanza(Presence::available().with_to(room_bob).with_payload(Muc::new()).into()).await.unwrap();
    let seen_by_alice = wait(&mut alice, 10, |s| match s {
        Stanza::Presence(p) if p.from.as_ref().map(|f| f.to_string()) == Some(format!("{ROOM}/bob")) => Some(muc_jids(p)),
        _ => None,
    }).await;
    step("3 alice reads bob's real JID from the room presence", seen_by_alice.as_ref().map_or(false, |v| v.iter().any(|j| j.starts_with("bob@localhost/"))), format!("{seen_by_alice:?}"));

    // 4. message, correction, reaction
    let mut m1 = Message::new_with_type(MessageType::Groupchat, Some(Jid::from_str(ROOM).unwrap()));
    m1.id = Some(xmpp_parsers::message::Id("m1".to_string()));
    m1.bodies.insert(Default::default(), "hello".into());
    alice.send_stanza(m1.into()).await.unwrap();
    let got = wait(&mut bob, 10, |s| match s {
        Stanza::Message(m) if m.bodies.values().any(|b| b == "hello") => Some(m.id.clone().map(|i| i.0)),
        _ => None,
    }).await;
    step("4a groupchat message delivered", got.is_some(), format!("{got:?}"));
    let mut m2 = Message::new_with_type(MessageType::Groupchat, Some(Jid::from_str(ROOM).unwrap()));
    m2.id = Some(xmpp_parsers::message::Id("m2".to_string()));
    m2.bodies.insert(Default::default(), "hello (fixed)".into());
    m2.payloads.push(Replace { id: xmpp_parsers::message::Id("m1".to_string()) }.into());
    alice.send_stanza(m2.into()).await.unwrap();
    let corr = wait(&mut bob, 10, |s| match s {
        Stanza::Message(m) => m.payloads.iter().find_map(|el| Replace::try_from(el.clone()).ok()).map(|r| (r.id.0.clone(), m.bodies.values().next().cloned())),
        _ => None,
    }).await;
    step("4b message correction (XEP-0308) sent and parsed", corr.as_ref().map_or(false, |c| c.0 == "m1"), format!("{corr:?}"));
    let mut m3 = Message::new_with_type(MessageType::Groupchat, Some(Jid::from_str(ROOM).unwrap()));
    m3.id = Some(xmpp_parsers::message::Id("m3".to_string()));
    m3.payloads.push(Reactions { id: "m1".into(), reactions: vec![Reaction { emoji: "\u{1F44D}".into() }] }.into());
    alice.send_stanza(m3.into()).await.unwrap();
    let react = wait(&mut bob, 10, |s| match s {
        Stanza::Message(m) => m.payloads.iter().find_map(|el| Reactions::try_from(el.clone()).ok()).map(|r| (r.id, r.reactions.len())),
        _ => None,
    }).await;
    step("4c reaction (XEP-0444) sent and parsed", react.as_ref().map_or(false, |r| r.0 == "m1" && r.1 == 1), format!("{react:?}"));

    // 5. archive: five more bodied messages, then page through the room history two at a time
    for i in 1..=5 {
        let mut m = Message::new_with_type(MessageType::Groupchat, Some(Jid::from_str(ROOM).unwrap()));
        m.bodies.insert(Default::default(), format!("filler {i}"));
        alice.send_stanza(m.into()).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut after: Option<String> = None;
    let mut total = 0;
    let mut pages = 0;
    for page in 0..4 {
        let q = mam::Query { queryid: Some(mam::QueryId(format!("q{page}"))), node: None, form: None,
            set: Some(SetQuery { max: Some(2), after: after.clone(), before: None, index: None }), flip_page: false };
        let iq = Iq::from_set(format!("mam{page}"), q).with_to(Jid::from_str(ROOM).unwrap());
        bob.send_stanza(iq.into()).await.unwrap();
        let want = format!("mam{page}");
        let mut results = 0;
        let fin = wait(&mut bob, 10, |s| match s {
            Stanza::Message(m) => { if m.payloads.iter().any(|el| mam::Result_::try_from(el.clone()).is_ok()) { results += 1; } None }
            Stanza::Iq(Iq::Result { id, payload: Some(el), .. }) if *id == want => mam::Fin::try_from(el.clone()).ok(),
            _ => None,
        }).await;
        match fin {
            Some(f) => { total += results; pages += 1; after = f.set.last.clone(); if f.complete { break; } }
            None => { step("5 archive query (XEP-0313) with paging (RSM)", false, format!("no fin on page {page}")); break; }
        }
    }
    step("5 archive query (XEP-0313) with paging (RSM)", pages >= 3 && total == 7, format!("{total} archived messages over {pages} pages (expected 7: m1, the correction, 5 fillers; the body-less reaction is not archived)"));

    // 6. file upload (XEP-0363) and download
    use xmpp_parsers::http_upload::{SlotRequest, SlotResult};
    let body = b"spike payload 123".to_vec();
    let req = SlotRequest { filename: "spike.txt".into(), size: body.len() as u64, content_type: Some("text/plain".into()) };
    bob.send_stanza(Iq::from_get("slot1", req).with_to(Jid::from_str("upload.localhost").unwrap()).into()).await.unwrap();
    let slot = wait(&mut bob, 10, |s| match s {
        Stanza::Iq(Iq::Result { id, payload: Some(el), .. }) if id == "slot1" => SlotResult::try_from(el.clone()).ok(),
        _ => None,
    }).await;
    match slot {
        Some(s) => {
            let http = reqwest::Client::new();
            let mut put = http.put(&s.put.url).body(body.clone());
            for h in &s.put.headers { put = put.header(h.name.as_str(), h.value.clone()); }
            let put_ok = put.send().await.map(|r| r.status().as_u16()).unwrap_or(0);
            let got = http.get(&s.get.url).send().await.ok();
            let same = match got { Some(r) => r.bytes().await.map(|b| b.to_vec() == body).unwrap_or(false), None => false };
            step("6 upload slot, PUT and GET round trip", (put_ok == 201 || put_ok == 200) && same, format!("put status {put_ok}, content identical: {same}"));
        }
        None => step("6 upload slot", false, "no slot result"),
    }
    let _ = alice.send_end().await;
    let _ = bob.send_end().await;
    let fails = FAILS.load(Ordering::SeqCst);
    println!("{}", if fails == 0 { "ALL CHECKS PASSED" } else { "SOME CHECKS FAILED" });
    std::process::exit(if fails == 0 { 0 } else { 1 });
}
