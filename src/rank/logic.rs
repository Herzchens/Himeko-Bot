use crate::config::RankConfig;

pub fn format_nickname(config: &RankConfig, level: u8) -> anyhow::Result<String> {
    let (rank_name, stars) = config.level_to_display(level)?;
    Ok(format!("{} {} SAO", rank_name, stars))
}

pub fn parse_nickname(config: &RankConfig, nick: &str) -> Option<u8> {
    let nick_upper = nick.to_uppercase();
    for (i, rank) in config.ranks.iter().enumerate() {
        if nick_upper.contains(&rank.to_uppercase()) {
            for s in 1..=config.stars_per_rank {
                if nick_upper.contains(&format!("{} SAO", s)) {
                    return Some(i as u8 * config.stars_per_rank + s);
                }
            }
            return Some(i as u8 * config.stars_per_rank + 1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_config() -> RankConfig {
        RankConfig {
            enabled: true,
            guild_id: 1,
            target_role_id: 1,
            leaderboard_channel_id: 1,
            stars_per_rank: 3,
            ranks: vec![
                "CHÓ NÁT".to_string(),
                "CHÓ RÁCH TẬP SỰ".to_string(),
            ],
        }
    }

    #[test]
    fn test_format_nickname() {
        let config = mock_config();
        assert_eq!(format_nickname(&config, 1).unwrap(), "CHÓ NÁT 1 SAO");
        assert_eq!(format_nickname(&config, 3).unwrap(), "CHÓ NÁT 3 SAO");
        assert_eq!(format_nickname(&config, 4).unwrap(), "CHÓ RÁCH TẬP SỰ 1 SAO");
        assert!(format_nickname(&config, 7).is_err());
    }

    #[test]
    fn test_parse_nickname() {
        let config = mock_config();
        assert_eq!(parse_nickname(&config, "CHÓ NÁT 2 SAO"), Some(2));
        assert_eq!(parse_nickname(&config, "CHÓ RÁCH TẬP SỰ 3 SAO"), Some(6));
        assert_eq!(parse_nickname(&config, "chó nát"), Some(1));
        assert_eq!(parse_nickname(&config, "Bình thường"), None);
    }
}
