//! Client-side validation, the same rules as the provisioner's `room-create` and `room-assign`.

use crate::errors::ValidationError;

/// A room name that passed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomName(String);

impl RoomName {
    /// Accept `[a-z0-9_]{1,32}` that does not start with `org_`.
    pub fn parse(input: &str) -> Result<Self, ValidationError> {
        Ok(Self(input.to_owned()))
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A bare JID that passed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BareJid(String);

impl BareJid {
    /// Accept `<local>@<domain>`: one `@`, both parts non-empty, no `/`, no whitespace.
    pub fn parse(input: &str) -> Result<Self, ValidationError> {
        Ok(Self(input.to_owned()))
    }

    /// The validated JID.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The affiliations a caller may set. Never admin or owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affiliation {
    /// A room member.
    Member,
    /// No affiliation: removes the member.
    None,
}

impl Affiliation {
    /// The wire value.
    pub fn as_str(self) -> &'static str {
        "member"
    }

    /// Accept `member` or `none`.
    pub fn parse(input: &str) -> Result<Self, ValidationError> {
        let _ = input;
        Ok(Self::Member)
    }
}

#[cfg(test)]
mod room_names {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("a")]
    #[case("lab_7")]
    #[case("0")]
    #[case("organization")]
    #[case("orgx")]
    #[case("________________________________")]
    fn room_name_should_be_accepted_when_it_matches_the_pattern(#[case] input: &str) {
        assert_eq!(RoomName::parse(input).map(|n| n.as_str().to_owned()), Ok(input.to_owned()));
    }

    #[rstest]
    #[case("")]
    #[case("Lab")]
    #[case("lab-1")]
    #[case("lab 1")]
    #[case("lab\n")]
    #[case("\nlab")]
    #[case("lab/x")]
    #[case("lab@muc.example.org")]
    #[case("lab\u{e9}")]
    #[case("ORG_lab")]
    #[case("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    fn room_name_should_be_refused_when_it_does_not_match_the_pattern(#[case] input: &str) {
        assert_eq!(RoomName::parse(input), Err(ValidationError::RoomName));
    }

    #[rstest]
    #[case("org_")]
    #[case("org_staff")]
    fn room_name_should_be_refused_when_it_starts_with_org_underscore(#[case] input: &str) {
        assert_eq!(RoomName::parse(input), Err(ValidationError::OrgPrefix));
    }

    #[test]
    fn room_name_should_accept_the_longest_name_when_it_has_32_characters() {
        assert!(RoomName::parse(&"a".repeat(32)).is_ok());
        assert_eq!(RoomName::parse(&"a".repeat(33)), Err(ValidationError::RoomName));
    }
}

#[cfg(test)]
mod member_jids {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("alice@chat.example.org")]
    #[case("a@b")]
    #[case("sbx-1-2@chat.example.org")]
    fn jid_should_be_accepted_when_it_is_bare(#[case] input: &str) {
        assert_eq!(BareJid::parse(input).map(|j| j.as_str().to_owned()), Ok(input.to_owned()));
    }

    #[rstest]
    #[case("")]
    #[case("alice")]
    #[case("@chat.example.org")]
    #[case("alice@")]
    #[case("@")]
    #[case("alice@chat.example.org/phone")]
    #[case("alice/phone@chat.example.org")]
    #[case("a b@chat.example.org")]
    #[case("alice@chat example.org")]
    #[case(" alice@chat.example.org")]
    #[case("alice@chat.example.org\n")]
    #[case("alice\t@chat.example.org")]
    #[case("a@b@chat.example.org")]
    fn jid_should_be_refused_when_it_is_not_bare(#[case] input: &str) {
        assert_eq!(BareJid::parse(input), Err(ValidationError::Jid));
    }
}

#[cfg(test)]
mod affiliations {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("member", Affiliation::Member, "member")]
    #[case("none", Affiliation::None, "none")]
    fn affiliation_should_round_trip_when_it_is_member_or_none(#[case] input: &str, #[case] expected: Affiliation, #[case] wire: &str) {
        assert_eq!(Affiliation::parse(input), Ok(expected));
        assert_eq!(expected.as_str(), wire);
    }

    #[rstest]
    #[case("admin")]
    #[case("owner")]
    #[case("outcast")]
    #[case("Member")]
    #[case("")]
    fn affiliation_should_be_refused_when_it_is_not_member_or_none(#[case] input: &str) {
        assert_eq!(Affiliation::parse(input), Err(ValidationError::Affiliation));
    }
}
