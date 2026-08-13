use crate::config::GuildRankConfig;

pub fn format_nickname(config: &GuildRankConfig, level: u8) -> anyhow::Result<String> {
    let (rank_name, stars) = config.level_to_display(level)?;
    Ok(format!("{rank_name} {stars} SAO"))
}

fn starts_with_boundary(text: &str, prefix: &str) -> bool {
    if !text.starts_with(prefix) {
        return false;
    }
    text[prefix.len()..].chars().next().is_none_or(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '|' | '-' | '–' | '—' | '•' | '·' | '/' | '\\' | '(' | '['
            )
    })
}

pub fn parse_nickname(config: &GuildRankConfig, nick: &str) -> Option<u8> {
    let nick_upper = nick.trim_start().to_uppercase();
    let mut best: Option<(usize, u8)> = None;

    for level in 1..=config.max_level() {
        let prefix = format_nickname(config, level).ok()?.to_uppercase();
        if starts_with_boundary(&nick_upper, &prefix)
            && best.is_none_or(|(best_len, _)| prefix.len() > best_len)
        {
            best = Some((prefix.len(), level));
        }
    }

    for (index, rank) in config.ranks.iter().enumerate() {
        let prefix = rank.trim().to_uppercase();
        if prefix.is_empty() || !starts_with_boundary(&nick_upper, &prefix) {
            continue;
        }
        let base = index.checked_mul(config.stars_per_rank as usize)?;
        let level = u8::try_from(base + 1).ok()?;
        if best.is_none_or(|(best_len, _)| prefix.len() > best_len) {
            best = Some((prefix.len(), level));
        }
    }

    best.map(|(_, level)| level)
}

pub fn managed_suffix<'a>(config: &GuildRankConfig, nick: &'a str, level: u8) -> Option<&'a str> {
    let prefix = format_nickname(config, level).ok()?;
    if starts_with_boundary(nick, &prefix) {
        Some(&nick[prefix.len()..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GuildRankConfig {
        GuildRankConfig {
            enabled: true,
            target_role_id: 1,
            leaderboard_channel_id: 2,
            stars_per_rank: 3,
            ranks: vec!["CHÓ NÁT".into(), "CHÓ RÁCH TẬP SỰ".into()],
        }
    }

    #[test]
    fn format_and_parse_respect_rank_boundaries() {
        let config = config();
        assert_eq!(format_nickname(&config, 1).unwrap(), "CHÓ NÁT 1 SAO");
        assert_eq!(format_nickname(&config, 3).unwrap(), "CHÓ NÁT 3 SAO");
        assert_eq!(
            format_nickname(&config, 4).unwrap(),
            "CHÓ RÁCH TẬP SỰ 1 SAO"
        );
        assert!(format_nickname(&config, 7).is_err());
        assert_eq!(parse_nickname(&config, "CHÓ NÁT 2 SAO"), Some(2));
        assert_eq!(parse_nickname(&config, "chó rách tập sự 3 sao"), Some(6));
        assert_eq!(parse_nickname(&config, "CHÓ NÁT | Bình"), Some(1));
        assert_eq!(parse_nickname(&config, "Bình thường"), None);
    }

    #[test]
    fn parser_round_trips_multi_digit_stars_and_ignores_suffix_tokens() {
        let config = GuildRankConfig {
            enabled: true,
            target_role_id: 1,
            leaderboard_channel_id: 2,
            stars_per_rank: 12,
            ranks: vec!["BRONZE".into(), "BRONZE PRO".into()],
        };

        for level in 1..=config.max_level() {
            let formatted = format_nickname(&config, level).unwrap();
            assert_eq!(parse_nickname(&config, &formatted), Some(level));
            assert_eq!(
                parse_nickname(&config, &format!("{formatted} | thích 1 SAO | ex-BRONZE")),
                Some(level)
            );
        }
        assert_eq!(parse_nickname(&config, "BRONZE PRO | Alice"), Some(13));
    }

    #[test]
    fn managed_suffix_uses_the_same_boundary_rule_as_rank_parsing() {
        let config = GuildRankConfig {
            enabled: true,
            target_role_id: 1,
            leaderboard_channel_id: 2,
            stars_per_rank: 3,
            ranks: vec!["BRONZE".into()],
        };

        assert_eq!(managed_suffix(&config, "BRONZE 2 SAO", 2), Some(""));
        assert_eq!(
            managed_suffix(&config, "BRONZE 2 SAO | custom", 2),
            Some(" | custom")
        );
        assert_eq!(
            managed_suffix(&config, "BRONZE 2 SAO - legacy", 2),
            Some(" - legacy")
        );
        assert_eq!(managed_suffix(&config, "BRONZE 2 SAOXYZ", 2), None);
        assert_eq!(managed_suffix(&config, "XBRONZE 2 SAO | custom", 2), None);
    }

    #[test]
    fn parser_requires_rank_at_the_start_of_the_managed_nickname() {
        let config = config();
        assert_eq!(parse_nickname(&config, "Alice | CHÓ NÁT 3 SAO"), None);
        assert_eq!(parse_nickname(&config, "XCHÓ NÁT 3 SAO"), None);
    }
}
