//! Pure stanza building and parsing: no I/O, no clock, ids are arguments.

use std::collections::HashMap;

use tokio_xmpp::Stanza;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::mam::Fin;
use xmpp_parsers::message::Message;
use xmpp_parsers::presence::Presence;

use crate::{Archived, Error, Event};

pub fn chat(_to: &str, _id: &str, _body: &str) -> Result<Message, Error> {
    Ok(Message::new(None))
}

pub fn groupchat(_room: &str, _id: &str, _body: &str) -> Result<Message, Error> {
    Ok(Message::new(None))
}

pub fn correction(
    _to: &str,
    _groupchat: bool,
    _id: &str,
    _replaces: &str,
    _body: &str,
) -> Result<Message, Error> {
    Ok(Message::new(None))
}

pub fn reaction(
    _to: &str,
    _groupchat: bool,
    _id: &str,
    _target: &str,
    _emojis: &[&str],
) -> Result<Message, Error> {
    Ok(Message::new(None))
}

pub fn chat_state(_to: &str, _id: &str, _state: &str) -> Result<Message, Error> {
    Ok(Message::new(None))
}

pub fn join(_room: &str, _nick: &str) -> Result<Presence, Error> {
    Ok(Presence::available())
}

pub fn initial_presence() -> Presence {
    Presence::new(xmpp_parsers::presence::Type::Unavailable)
}

pub fn history_query(
    _id: &str,
    _queryid: &str,
    _with: Option<&str>,
    _room: bool,
    _limit: usize,
    _before: Option<&str>,
) -> Result<Iq, Error> {
    Ok(Iq::Result {
        from: None,
        to: None,
        id: String::new(),
        payload: None,
    })
}

pub fn slot_request(
    _host: &str,
    _id: &str,
    _filename: &str,
    _size: u64,
    _content_type: &str,
) -> Result<Iq, Error> {
    Ok(Iq::Result {
        from: None,
        to: None,
        id: String::new(),
        payload: None,
    })
}

/// The archive entry and the query id of a MAM result message, or `None` when the message is not one.
pub fn mam_result(_m: &Message) -> Option<(String, Archived)> {
    None
}

/// The query id of the `<fin/>` of an archive query is not in the stanza; the iq id is. Returns the iq id and the fin.
pub fn mam_fin(_iq: &Iq) -> Option<(String, Fin)> {
    None
}

/// The reply to an iq the server sent us: an empty result to a ping, an error to every other get or set, nothing to a result.
pub fn iq_reply(_iq: &Iq) -> Option<Iq> {
    None
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

    pub fn handle(&mut self, _stanza: &Stanza) -> Vec<Event> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xmpp_parsers::minidom::Element;

    fn el(xml: &str) -> Element {
        xml.parse().unwrap()
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
            "<presence xmlns='jabber:client' to='room@rooms.example.org/bot'><x xmlns='http://jabber.org/protocol/muc'><history maxstanzas='0'/></x></presence>",
        );
        assert_eq!(got, want);
    }

    #[test]
    fn initial_presence_is_plain_available() {
        assert_eq!(
            Element::from(initial_presence()),
            el("<presence xmlns='jabber:client'/>")
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
        let got = Element::from(history_query("h2", "q2", Some(ROOM), true, 5, None).unwrap());
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
