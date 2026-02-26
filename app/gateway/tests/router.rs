//! Channel routing tests.

use agent::Platform;
use compact_str::CompactString;
use walrus_gateway::{ChannelRouter, RoutingRule};

#[test]
fn exact_match_priority() {
    let rules = vec![
        RoutingRule {
            platform: Platform::Telegram,
            channel_id: Some(CompactString::new("chan-123")),
            agent: CompactString::new("specific-agent"),
        },
        RoutingRule {
            platform: Platform::Telegram,
            channel_id: None,
            agent: CompactString::new("catchall-agent"),
        },
    ];
    let router = ChannelRouter::new(rules, None);
    let agent = router.route(Platform::Telegram, "chan-123").unwrap();
    assert_eq!(agent.as_str(), "specific-agent");
}

#[test]
fn platform_catchall() {
    let rules = vec![RoutingRule {
        platform: Platform::Telegram,
        channel_id: None,
        agent: CompactString::new("tg-agent"),
    }];
    let router = ChannelRouter::new(rules, None);
    let agent = router.route(Platform::Telegram, "any-channel").unwrap();
    assert_eq!(agent.as_str(), "tg-agent");
}

#[test]
fn default_fallback() {
    let router = ChannelRouter::new(vec![], Some(CompactString::new("default")));
    let agent = router.route(Platform::Telegram, "any").unwrap();
    assert_eq!(agent.as_str(), "default");
}

#[test]
fn no_match_no_default() {
    let router = ChannelRouter::new(vec![], None);
    assert!(router.route(Platform::Telegram, "any").is_none());
}

#[test]
fn exact_beats_catchall() {
    let rules = vec![
        RoutingRule {
            platform: Platform::Telegram,
            channel_id: None,
            agent: CompactString::new("catchall"),
        },
        RoutingRule {
            platform: Platform::Telegram,
            channel_id: Some(CompactString::new("special")),
            agent: CompactString::new("exact"),
        },
    ];
    let router = ChannelRouter::new(rules, Some(CompactString::new("default")));
    // Exact match wins
    let agent = router.route(Platform::Telegram, "special").unwrap();
    assert_eq!(agent.as_str(), "exact");
    // Non-matching falls to catchall
    let agent = router.route(Platform::Telegram, "other").unwrap();
    assert_eq!(agent.as_str(), "catchall");
}

#[test]
fn parse_platform_valid() {
    use walrus_gateway::router::parse_platform;
    let p = parse_platform("telegram").unwrap();
    assert_eq!(p, Platform::Telegram);
    let p = parse_platform("Telegram").unwrap();
    assert_eq!(p, Platform::Telegram);
}

#[test]
fn parse_platform_invalid() {
    use walrus_gateway::router::parse_platform;
    assert!(parse_platform("unknown").is_err());
}
