//! Pure stanza building and parsing: no I/O, no clock, ids are arguments.

use std::collections::HashMap;

use tokio_xmpp::Stanza;
use xmpp_parsers::chatstates::ChatState;
use xmpp_parsers::data_forms::{DataForm, DataFormType, Field, FieldType};
use xmpp_parsers::http_upload::SlotRequest;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::jid::Jid;
use xmpp_parsers::mam::{self, Fin};
use xmpp_parsers::message::{Id, Lang, Message, MessageType};
use xmpp_parsers::message_correct::Replace;
use xmpp_parsers::muc::muc::History;
use xmpp_parsers::muc::{Muc, MucUser};
use xmpp_parsers::presence::{Presence, Type as PresenceType};
use xmpp_parsers::reactions::{Reaction, Reactions};
use xmpp_parsers::rsm::SetQuery;
use xmpp_parsers::stanza_error::{DefinedCondition, ErrorType, StanzaError};

use crate::{Archived, Error, Event};

fn jid(s: &str) -> Result<Jid, Error> {
    s.parse()
        .map_err(|e| Error::InvalidArgument(format!("{s:?}: {e}")))
}

fn message(kind: MessageType, to: &str, id: &str) -> Result<Message, Error> {
    let mut m = Message::new_with_type(kind, Some(jid(to)?));
    m.id = Some(Id(id.to_string()));
    Ok(m)
}

fn kind(groupchat: bool) -> MessageType {
    if groupchat {
        MessageType::Groupchat
    } else {
        MessageType::Chat
    }
}

pub fn chat(to: &str, id: &str, body: &str) -> Result<Message, Error> {
    let mut m = message(MessageType::Chat, to, id)?;
    m.bodies.insert(Lang::default(), body.to_string());
    Ok(m)
}

pub fn groupchat(room: &str, id: &str, body: &str) -> Result<Message, Error> {
    let mut m = message(MessageType::Groupchat, room, id)?;
    m.bodies.insert(Lang::default(), body.to_string());
    Ok(m)
}

/// A message with `id` that replaces the earlier message `replaces` (XEP-0308).
pub fn correction(
    to: &str,
    groupchat: bool,
    id: &str,
    replaces: &str,
    body: &str,
) -> Result<Message, Error> {
    let mut m = message(kind(groupchat), to, id)?;
    m.bodies.insert(Lang::default(), body.to_string());
    m.payloads.push(
        Replace {
            id: Id(replaces.to_string()),
        }
        .into(),
    );
    Ok(m)
}

pub fn reaction(
    to: &str,
    groupchat: bool,
    id: &str,
    target: &str,
    emojis: &[&str],
) -> Result<Message, Error> {
    let mut m = message(kind(groupchat), to, id)?;
    let reactions = emojis
        .iter()
        .map(|e| Reaction {
            emoji: e.to_string(),
        })
        .collect();
    m.payloads.push(
        Reactions {
            id: target.to_string(),
            reactions,
        }
        .into(),
    );
    Ok(m)
}

/// A chat state notification (XEP-0085) to a one-to-one peer.
pub fn chat_state(to: &str, id: &str, state: &str) -> Result<Message, Error> {
    let state = match state {
        "active" => ChatState::Active,
        "composing" => ChatState::Composing,
        "paused" => ChatState::Paused,
        "inactive" => ChatState::Inactive,
        "gone" => ChatState::Gone,
        other => {
            return Err(Error::InvalidArgument(format!(
                "unknown chat state {other:?}"
            )));
        }
    };
    let mut m = message(MessageType::Chat, to, id)?;
    m.payloads.push(state.into());
    Ok(m)
}

/// Join a room as `nick`, asking for no history: the archive has it, and a replay would look like live messages.
pub fn join(room: &str, nick: &str) -> Result<Presence, Error> {
    if nick.is_empty() {
        return Err(Error::InvalidArgument("empty nick".into()));
    }
    let to = jid(&format!("{room}/{nick}"))?;
    Ok(Presence::available()
        .with_to(to)
        .with_payload(Muc::new().with_history(History::new().with_maxstanzas(0))))
}

pub fn initial_presence() -> Presence {
    Presence::available()
}

/// An archive query (XEP-0313) for the newest `limit` messages before the archive id `before` (the newest overall when `None`).
/// A room is queried on the room itself; a peer is a filter on our own archive.
pub fn history_query(
    id: &str,
    queryid: &str,
    with: Option<&str>,
    room: bool,
    limit: usize,
    before: Option<&str>,
) -> Result<Iq, Error> {
    let (to, form) = match (room, with) {
        (true, Some(room)) => (Some(jid(room)?), None),
        (true, None) => {
            return Err(Error::InvalidArgument(
                "a room history needs the room".into(),
            ));
        }
        (false, None) => (None, None),
        (false, Some(peer)) => {
            let peer = jid(peer)?;
            let mut field = Field::new("with", FieldType::TextSingle);
            field.values.push(peer.to_string());
            (
                None,
                Some(DataForm::new(
                    DataFormType::Submit,
                    "urn:xmpp:mam:2",
                    vec![field],
                )),
            )
        }
    };
    let query = mam::Query {
        queryid: Some(mam::QueryId(queryid.to_string())),
        node: None,
        form,
        // an empty <before/> asks for the last page
        set: Some(SetQuery {
            max: Some(limit),
            after: None,
            before: Some(before.unwrap_or("").to_string()),
            index: None,
        }),
        flip_page: false,
    };
    let iq = Iq::from_set(id, query);
    Ok(match to {
        Some(to) => iq.with_to(to),
        None => iq,
    })
}

pub fn slot_request(
    host: &str,
    id: &str,
    filename: &str,
    size: u64,
    content_type: &str,
) -> Result<Iq, Error> {
    let req = SlotRequest {
        filename: filename.to_string(),
        size,
        content_type: Some(content_type.to_string()),
    };
    Ok(Iq::from_get(id, req).with_to(jid(host)?))
}

/// The archive entry and the query id of a MAM result message, or `None` when the message is not one or has no body.
pub fn mam_result(m: &Message) -> Option<(String, Archived)> {
    let r = m
        .payloads
        .iter()
        .find_map(|el| mam::Result_::try_from(el.clone()).ok())?;
    let inner = &r.forwarded.message;
    let body = inner
        .bodies
        .get(&Lang::default())
        .or_else(|| inner.bodies.values().next())?
        .clone();
    Some((
        r.queryid?.0,
        Archived {
            id: r.id,
            from: inner
                .from
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            body,
            stamp: r
                .forwarded
                .delay
                .map(|d| d.stamp.0.to_rfc3339())
                .unwrap_or_default(),
        },
    ))
}

/// The iq id and the `<fin/>` of a result that ends an archive query.
pub fn mam_fin(iq: &Iq) -> Option<(String, Fin)> {
    match iq {
        Iq::Result {
            id,
            payload: Some(el),
            ..
        } => Fin::try_from(el.clone()).ok().map(|f| (id.clone(), f)),
        _ => None,
    }
}

/// How a room join ended, if `p` is the answer: the room echoes our own presence on success, or sends an error.
pub fn join_result(p: &Presence, room: &str, nick: &str) -> Option<Result<(), String>> {
    if p.from.as_ref().map(ToString::to_string) != Some(format!("{room}/{nick}"))
        && p.from.as_ref().map(|f| f.to_bare().to_string()) != Some(room.to_string())
    {
        return None;
    }
    match p.type_ {
        PresenceType::Error => Some(Err(format!("{room}: join refused"))),
        PresenceType::None
            if p.from
                .as_ref()
                .and_then(|f| f.resource())
                .is_some_and(|r| r.as_str() == nick) =>
        {
            Some(Ok(()))
        }
        _ => None,
    }
}

/// The reply to an iq the server sent us: an empty result to a ping, an error to every other get or set, nothing to a result.
pub fn iq_reply(iq: &Iq) -> Option<Iq> {
    let (from, id, payload) = match iq {
        Iq::Get {
            from, id, payload, ..
        }
        | Iq::Set {
            from, id, payload, ..
        } => (from, id, payload),
        _ => return None,
    };
    let to = from.clone()?;
    if payload.is("ping", "urn:xmpp:ping") {
        return Some(Iq::empty_result(to, id.clone()));
    }
    let error = StanzaError::new(
        ErrorType::Cancel,
        DefinedCondition::ServiceUnavailable,
        "en",
        "",
    );
    Some(Iq::from_error(id.clone(), error).with_to(to))
}

/// Turns incoming stanzas into events and remembers the room state that needs: our nick per room and the real JID per occupant.
#[derive(Debug, Default)]
pub struct Parser {
    nicks: HashMap<String, String>,
    occupants: HashMap<(String, String), String>,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the nick we use in a room, so a message from it is `from_self`.
    pub fn set_nick(&mut self, room: &str, nick: &str) {
        self.nicks.insert(room.to_string(), nick.to_string());
    }

    pub fn handle(&mut self, stanza: &Stanza) -> Vec<Event> {
        match stanza {
            Stanza::Message(m) => self.message(m),
            Stanza::Presence(p) => self.presence(p),
            Stanza::Iq(_) => Vec::new(),
        }
    }

    fn presence(&mut self, p: &Presence) -> Vec<Event> {
        let available = match p.type_ {
            PresenceType::None => true,
            PresenceType::Unavailable => false,
            _ => return Vec::new(),
        };
        let Some(from) = &p.from else {
            return Vec::new();
        };
        if let Some(nick) = from.resource() {
            let key = (from.to_bare().to_string(), nick.to_string());
            let real = p
                .payloads
                .iter()
                .filter_map(|el| MucUser::try_from(el.clone()).ok())
                .flat_map(|u| u.items)
                .find_map(|i| i.jid);
            match (available, real) {
                (true, Some(real)) => {
                    self.occupants.insert(key, real.to_string());
                }
                (false, _) => {
                    self.occupants.remove(&key);
                }
                _ => {}
            }
        }
        vec![Event::Presence {
            from: from.to_string(),
            available,
        }]
    }

    fn message(&mut self, m: &Message) -> Vec<Event> {
        if matches!(m.type_, MessageType::Error | MessageType::Headline) || mam_result(m).is_some()
        {
            return Vec::new();
        }
        let Some(from) = &m.from else {
            return Vec::new();
        };
        let from_str = from.to_string();
        let room_nick = if m.type_ == MessageType::Groupchat {
            match from.resource() {
                Some(nick) => Some((from.to_bare().to_string(), nick.to_string())),
                None => return Vec::new(), // the room itself: a subject or a notice
            }
        } else {
            None
        };
        let mut events = Vec::new();
        let body = m
            .bodies
            .get(&Lang::default())
            .or_else(|| m.bodies.values().next());
        if let Some(body) = body {
            let id = m.id.as_ref().map(|i| i.0.clone());
            let replaces = m
                .payloads
                .iter()
                .find_map(|el| Replace::try_from(el.clone()).ok())
                .map(|r| r.id.0);
            events.push(match &room_nick {
                Some((room, nick)) => Event::Groupchat {
                    room: room.clone(),
                    nick: nick.clone(),
                    real_jid: self.occupants.get(&(room.clone(), nick.clone())).cloned(),
                    id,
                    body: body.clone(),
                    replaces,
                    from_self: self.nicks.get(room) == Some(nick),
                },
                None => Event::Chat {
                    from: from_str.clone(),
                    id,
                    body: body.clone(),
                    replaces,
                },
            });
        }
        if let Some(r) = m
            .payloads
            .iter()
            .find_map(|el| Reactions::try_from(el.clone()).ok())
        {
            events.push(Event::Reaction {
                from: from_str.clone(),
                target_id: r.id,
                emojis: r.reactions.into_iter().map(|e| e.emoji).collect(),
            });
        }
        if let Some(state) = m
            .payloads
            .iter()
            .find_map(|el| ChatState::try_from(el.clone()).ok())
        {
            let state = match state {
                ChatState::Active => "active",
                ChatState::Composing => "composing",
                ChatState::Paused => "paused",
                ChatState::Inactive => "inactive",
                ChatState::Gone => "gone",
            };
            events.push(Event::ChatState {
                from: from_str,
                state: state.to_string(),
            });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xmpp_parsers::minidom::Element;

    fn el(xml: &str) -> Element {
        xml.parse().unwrap()
    }

    /// The same element after a trip through text: an empty `<before></before>` is written with an empty text node in memory.
    fn wire(e: Element) -> Element {
        String::from(&e).parse().unwrap()
    }

    fn message(xml: &str) -> Stanza {
        Stanza::Message(Message::try_from(el(xml)).unwrap())
    }

    fn presence(xml: &str) -> Stanza {
        Stanza::Presence(Presence::try_from(el(xml)).unwrap())
    }

    fn iq(xml: &str) -> Iq {
        Iq::try_from(el(xml)).unwrap()
    }

    const ROOM: &str = "room@rooms.example.org";

    // ---- outgoing ----

    #[test]
    fn builds_a_chat_message() {
        let got = Element::from(chat("bob@example.org/phone", "m1", "hi").unwrap());
        let want = el(
            "<message xmlns='jabber:client' to='bob@example.org/phone' id='m1' type='chat'><body>hi</body></message>",
        );
        assert_eq!(got, want);
    }

    #[test]
    fn builds_a_groupchat_message() {
        let got = Element::from(groupchat(ROOM, "m2", "hello").unwrap());
        let want = el(
            "<message xmlns='jabber:client' to='room@rooms.example.org' id='m2' type='groupchat'><body>hello</body></message>",
        );
        assert_eq!(got, want);
    }

    #[test]
    fn builds_a_correction_for_chat_and_room() {
        let got = Element::from(correction("bob@example.org", false, "m3", "m1", "hi!").unwrap());
        let want = el(
            "<message xmlns='jabber:client' to='bob@example.org' id='m3' type='chat'><body>hi!</body><replace xmlns='urn:xmpp:message-correct:0' id='m1'/></message>",
        );
        assert_eq!(got, want);
        let got = Element::from(correction(ROOM, true, "m3", "m1", "hi!").unwrap());
        assert_eq!(got.attr("type"), Some("groupchat"));
    }

    #[test]
    fn builds_a_reaction_with_every_emoji() {
        let got =
            Element::from(reaction(ROOM, true, "r1", "m1", &["\u{1F44D}", "\u{2764}"]).unwrap());
        let want = el(
            "<message xmlns='jabber:client' to='room@rooms.example.org' id='r1' type='groupchat'><reactions xmlns='urn:xmpp:reactions:0' id='m1'><reaction>\u{1F44D}</reaction><reaction>\u{2764}</reaction></reactions></message>",
        );
        assert_eq!(got, want);
        let dm = Element::from(reaction("bob@example.org", false, "r2", "m1", &[]).unwrap());
        assert_eq!(dm.attr("type"), Some("chat"));
    }

    #[test]
    fn builds_a_chat_state_and_rejects_an_unknown_one() {
        let got = Element::from(chat_state("bob@example.org", "c1", "composing").unwrap());
        let want = el(
            "<message xmlns='jabber:client' to='bob@example.org' id='c1' type='chat'><composing xmlns='http://jabber.org/protocol/chatstates'/></message>",
        );
        assert_eq!(got, want);
        for s in ["active", "paused", "inactive", "gone"] {
            assert!(chat_state("bob@example.org", "c1", s).is_ok(), "{s}");
        }
        assert!(matches!(
            chat_state("bob@example.org", "c1", "typing"),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn rejects_an_invalid_jid() {
        assert!(matches!(
            chat("not a jid@@", "m1", "x"),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(join(ROOM, ""), Err(Error::InvalidArgument(_))));
    }

    #[test]
    fn builds_a_join_that_asks_for_no_history() {
        let got = Element::from(join(ROOM, "bot").unwrap());
        let want = el(
            "<presence xmlns='jabber:client' to='room@rooms.example.org/bot'><priority>0</priority><x xmlns='http://jabber.org/protocol/muc'><history maxstanzas='0'/></x></presence>",
        );
        assert_eq!(got, want);
    }

    #[test]
    fn initial_presence_is_plain_available() {
        assert_eq!(
            Element::from(initial_presence()),
            el("<presence xmlns='jabber:client'><priority>0</priority></presence>")
        );
    }

    #[test]
    fn builds_a_history_query_for_a_peer() {
        let got = Element::from(
            history_query(
                "h1",
                "q1",
                Some("bob@example.org"),
                false,
                20,
                Some("arch-7"),
            )
            .unwrap(),
        );
        let want = el(
            "<iq xmlns='jabber:client' type='set' id='h1'><query xmlns='urn:xmpp:mam:2' queryid='q1'><x xmlns='jabber:x:data' type='submit'><field var='FORM_TYPE' type='hidden'><value>urn:xmpp:mam:2</value></field><field var='with'><value>bob@example.org</value></field></x><set xmlns='http://jabber.org/protocol/rsm'><max>20</max><before>arch-7</before></set></query></iq>",
        );
        assert_eq!(got, want);
    }

    #[test]
    fn builds_a_history_query_for_the_latest_page_of_a_room() {
        let got = wire(Element::from(
            history_query("h2", "q2", Some(ROOM), true, 5, None).unwrap(),
        ));
        let want = el(
            "<iq xmlns='jabber:client' type='set' id='h2' to='room@rooms.example.org'><query xmlns='urn:xmpp:mam:2' queryid='q2'><set xmlns='http://jabber.org/protocol/rsm'><max>5</max><before/></set></query></iq>",
        );
        assert_eq!(got, want);
    }

    #[test]
    fn a_room_history_needs_a_room() {
        assert!(matches!(
            history_query("h", "q", None, true, 5, None),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn builds_an_upload_slot_request() {
        let got = Element::from(
            slot_request("upload.example.org", "u1", "a.txt", 17, "text/plain").unwrap(),
        );
        let want = el(
            "<iq xmlns='jabber:client' type='get' id='u1' to='upload.example.org'><request xmlns='urn:xmpp:http:upload:0' filename='a.txt' size='17' content-type='text/plain'/></iq>",
        );
        assert_eq!(got, want);
    }

    // ---- incoming ----

    #[test]
    fn parses_a_chat_message() {
        let mut p = Parser::new();
        let got = p.handle(&message("<message xmlns='jabber:client' from='bob@example.org/phone' id='m1' type='chat'><body>hi</body></message>"));
        assert_eq!(
            got,
            vec![Event::Chat {
                from: "bob@example.org/phone".into(),
                id: Some("m1".into()),
                body: "hi".into(),
                replaces: None
            }]
        );
    }

    #[test]
    fn parses_a_correction() {
        let mut p = Parser::new();
        let got = p.handle(&message("<message xmlns='jabber:client' from='bob@example.org/phone' id='m2' type='chat'><body>hi!</body><replace xmlns='urn:xmpp:message-correct:0' id='m1'/></message>"));
        assert_eq!(
            got,
            vec![Event::Chat {
                from: "bob@example.org/phone".into(),
                id: Some("m2".into()),
                body: "hi!".into(),
                replaces: Some("m1".into())
            }]
        );
    }

    #[test]
    fn a_normal_message_with_a_body_is_a_chat() {
        let mut p = Parser::new();
        let got = p.handle(&message(
            "<message xmlns='jabber:client' from='bob@example.org/phone'><body>hi</body></message>",
        ));
        assert!(matches!(got.as_slice(), [Event::Chat { id: None, .. }]));
    }

    #[test]
    fn parses_a_groupchat_message_with_the_real_jid_after_the_presence() {
        let mut p = Parser::new();
        let seen = p.handle(&presence("<presence xmlns='jabber:client' from='room@rooms.example.org/bob'><x xmlns='http://jabber.org/protocol/muc#user'><item affiliation='member' role='participant' jid='bob@example.org/phone'/></x></presence>"));
        assert_eq!(
            seen,
            vec![Event::Presence {
                from: "room@rooms.example.org/bob".into(),
                available: true
            }]
        );
        let got = p.handle(&message("<message xmlns='jabber:client' from='room@rooms.example.org/bob' id='g1' type='groupchat'><body>yo</body></message>"));
        assert_eq!(
            got,
            vec![Event::Groupchat {
                room: ROOM.into(),
                nick: "bob".into(),
                real_jid: Some("bob@example.org/phone".into()),
                id: Some("g1".into()),
                body: "yo".into(),
                replaces: None,
                from_self: false,
            }]
        );
    }

    #[test]
    fn a_groupchat_message_from_an_unseen_nick_has_no_real_jid_and_our_own_is_from_self() {
        let mut p = Parser::new();
        p.set_nick(ROOM, "bot");
        let got = p.handle(&message("<message xmlns='jabber:client' from='room@rooms.example.org/bot' type='groupchat'><body>mine</body></message>"));
        assert!(matches!(
            got.as_slice(),
            [Event::Groupchat {
                real_jid: None,
                from_self: true,
                ..
            }]
        ));
        let got = p.handle(&message("<message xmlns='jabber:client' from='room@rooms.example.org/eve' type='groupchat'><body>x</body></message>"));
        assert!(matches!(
            got.as_slice(),
            [Event::Groupchat {
                real_jid: None,
                from_self: false,
                ..
            }]
        ));
    }

    #[test]
    fn an_occupant_who_left_loses_the_real_jid() {
        let mut p = Parser::new();
        p.handle(&presence("<presence xmlns='jabber:client' from='room@rooms.example.org/bob'><x xmlns='http://jabber.org/protocol/muc#user'><item affiliation='member' role='participant' jid='bob@example.org/phone'/></x></presence>"));
        let gone = p.handle(&presence("<presence xmlns='jabber:client' from='room@rooms.example.org/bob' type='unavailable'><x xmlns='http://jabber.org/protocol/muc#user'><item affiliation='member' role='none'/></x></presence>"));
        assert_eq!(
            gone,
            vec![Event::Presence {
                from: "room@rooms.example.org/bob".into(),
                available: false
            }]
        );
        let got = p.handle(&message("<message xmlns='jabber:client' from='room@rooms.example.org/bob' type='groupchat'><body>x</body></message>"));
        assert!(matches!(
            got.as_slice(),
            [Event::Groupchat { real_jid: None, .. }]
        ));
    }

    #[test]
    fn parses_a_reaction_without_a_body() {
        let mut p = Parser::new();
        let got = p.handle(&message("<message xmlns='jabber:client' from='room@rooms.example.org/bob' type='groupchat'><reactions xmlns='urn:xmpp:reactions:0' id='m1'><reaction>\u{1F44D}</reaction></reactions></message>"));
        assert_eq!(
            got,
            vec![Event::Reaction {
                from: "room@rooms.example.org/bob".into(),
                target_id: "m1".into(),
                emojis: vec!["\u{1F44D}".into()]
            }]
        );
    }

    #[test]
    fn parses_a_chat_state() {
        let mut p = Parser::new();
        let got = p.handle(&message("<message xmlns='jabber:client' from='bob@example.org/phone' type='chat'><composing xmlns='http://jabber.org/protocol/chatstates'/></message>"));
        assert_eq!(
            got,
            vec![Event::ChatState {
                from: "bob@example.org/phone".into(),
                state: "composing".into()
            }]
        );
    }

    #[test]
    fn a_body_with_a_chat_state_gives_both_events() {
        let mut p = Parser::new();
        let got = p.handle(&message("<message xmlns='jabber:client' from='bob@example.org/phone' type='chat'><body>hi</body><active xmlns='http://jabber.org/protocol/chatstates'/></message>"));
        assert!(matches!(
            got.as_slice(),
            [Event::Chat { .. }, Event::ChatState { .. }]
        ));
    }

    #[test]
    fn parses_presence_going_on_and_offline() {
        let mut p = Parser::new();
        let on = p.handle(&presence(
            "<presence xmlns='jabber:client' from='bob@example.org/phone'/>",
        ));
        assert_eq!(
            on,
            vec![Event::Presence {
                from: "bob@example.org/phone".into(),
                available: true
            }]
        );
        let off = p.handle(&presence(
            "<presence xmlns='jabber:client' from='bob@example.org/phone' type='unavailable'/>",
        ));
        assert_eq!(
            off,
            vec![Event::Presence {
                from: "bob@example.org/phone".into(),
                available: false
            }]
        );
    }

    #[test]
    fn ignores_what_is_not_an_event() {
        let mut p = Parser::new();
        // an error bounce, a room subject, a presence subscription request, an archive result, a message from nobody
        for s in [
            message(
                "<message xmlns='jabber:client' from='bob@example.org' type='error'><body>x</body><error type='cancel'><service-unavailable xmlns='urn:ietf:params:xml:ns:xmpp-stanzas'/></error></message>",
            ),
            message(
                "<message xmlns='jabber:client' from='room@rooms.example.org' type='groupchat'><subject>topic</subject></message>",
            ),
            presence("<presence xmlns='jabber:client' from='bob@example.org' type='subscribe'/>"),
            message(
                "<message xmlns='jabber:client'><result xmlns='urn:xmpp:mam:2' queryid='q1' id='a1'><forwarded xmlns='urn:xmpp:forward:0'><delay xmlns='urn:xmpp:delay' stamp='2026-01-02T03:04:05Z'/><message xmlns='jabber:client' from='bob@example.org/phone' type='chat'><body>old</body></message></forwarded></result></message>",
            ),
            message("<message xmlns='jabber:client' from='bob@example.org/phone' type='chat'/>"),
        ] {
            assert_eq!(p.handle(&s), vec![]);
        }
    }

    // ---- archive ----

    #[test]
    fn extracts_an_archived_message_with_its_query_id() {
        let m = Message::try_from(el("<message xmlns='jabber:client' from='room@rooms.example.org'><result xmlns='urn:xmpp:mam:2' queryid='q1' id='a1'><forwarded xmlns='urn:xmpp:forward:0'><delay xmlns='urn:xmpp:delay' stamp='2026-01-02T03:04:05Z'/><message xmlns='jabber:client' from='room@rooms.example.org/bob' type='groupchat'><body>old</body></message></forwarded></result></message>")).unwrap();
        let (q, a) = mam_result(&m).unwrap();
        assert_eq!(q, "q1");
        assert_eq!(
            a,
            Archived {
                id: "a1".into(),
                from: "room@rooms.example.org/bob".into(),
                body: "old".into(),
                stamp: "2026-01-02T03:04:05+00:00".into()
            }
        );
    }

    #[test]
    fn an_archived_message_without_a_body_or_a_plain_message_is_not_a_result() {
        let nobody = Message::try_from(el("<message xmlns='jabber:client'><result xmlns='urn:xmpp:mam:2' queryid='q1' id='a1'><forwarded xmlns='urn:xmpp:forward:0'><delay xmlns='urn:xmpp:delay' stamp='2026-01-02T03:04:05Z'/><message xmlns='jabber:client' from='bob@example.org/phone' type='chat'/></forwarded></result></message>")).unwrap();
        assert!(mam_result(&nobody).is_none());
        let plain = Message::try_from(el(
            "<message xmlns='jabber:client'><body>x</body></message>",
        ))
        .unwrap();
        assert!(mam_result(&plain).is_none());
    }

    #[test]
    fn extracts_the_fin_of_an_archive_query() {
        let r = iq(
            "<iq xmlns='jabber:client' type='result' id='h1'><fin xmlns='urn:xmpp:mam:2' complete='true'><set xmlns='http://jabber.org/protocol/rsm'><first>a1</first><last>a2</last></set></fin></iq>",
        );
        let (id, fin) = mam_fin(&r).unwrap();
        assert_eq!(id, "h1");
        assert!(fin.complete);
        let other = iq("<iq xmlns='jabber:client' type='result' id='h1'/>");
        assert!(mam_fin(&other).is_none());
    }

    // ---- joining ----

    fn pres(xml: &str) -> Presence {
        Presence::try_from(el(xml)).unwrap()
    }

    #[test]
    fn the_room_echoing_our_presence_completes_the_join() {
        let p = pres("<presence xmlns='jabber:client' from='room@rooms.example.org/bot'/>");
        assert_eq!(join_result(&p, ROOM, "bot"), Some(Ok(())));
    }

    #[test]
    fn a_presence_error_from_the_room_fails_the_join() {
        let p = pres(
            "<presence xmlns='jabber:client' from='room@rooms.example.org/bot' type='error'><error type='cancel'><item-not-found xmlns='urn:ietf:params:xml:ns:xmpp-stanzas'/></error></presence>",
        );
        assert!(matches!(join_result(&p, ROOM, "bot"), Some(Err(_))));
        let bare = pres(
            "<presence xmlns='jabber:client' from='room@rooms.example.org' type='error'><error type='cancel'><forbidden xmlns='urn:ietf:params:xml:ns:xmpp-stanzas'/></error></presence>",
        );
        assert!(matches!(join_result(&bare, ROOM, "bot"), Some(Err(_))));
    }

    #[test]
    fn other_presences_do_not_answer_a_join() {
        for xml in [
            "<presence xmlns='jabber:client' from='room@rooms.example.org/bob'/>",
            "<presence xmlns='jabber:client' from='other@rooms.example.org/bot'/>",
            "<presence xmlns='jabber:client' from='room@rooms.example.org/bot' type='unavailable'/>",
            "<presence xmlns='jabber:client' from='room@rooms.example.org'/>",
        ] {
            assert_eq!(join_result(&pres(xml), ROOM, "bot"), None, "{xml}");
        }
    }

    // ---- iq from the server ----

    #[test]
    fn answers_a_ping_and_refuses_other_requests() {
        let ping = iq(
            "<iq xmlns='jabber:client' type='get' id='p1' from='example.org'><ping xmlns='urn:xmpp:ping'/></iq>",
        );
        assert_eq!(
            Element::from(iq_reply(&ping).unwrap()),
            el("<iq xmlns='jabber:client' type='result' id='p1' to='example.org'/>")
        );
        let other = iq(
            "<iq xmlns='jabber:client' type='set' id='x1' from='example.org'><thing xmlns='urn:example:thing'/></iq>",
        );
        let reply = Element::from(iq_reply(&other).unwrap());
        assert_eq!(reply.attr("type"), Some("error"));
        assert_eq!(reply.attr("id"), Some("x1"));
        let result = iq("<iq xmlns='jabber:client' type='result' id='r1'/>");
        assert!(iq_reply(&result).is_none());
    }
}
