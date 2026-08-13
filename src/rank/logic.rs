use crate::config::GuildRankConfig;

pub fn format_nickname(config: &GuildRankConfig, level: u8) -> anyhow::Result<String> {
    let (rank_name, stars) = config.level_to_display(level)?;
    Ok(format!("{rank_name} {stars} SAO"))
}

pub fn parse_nickname(config: &GuildRankConfig, nick: &str) -> Option<u8> {
    let nick_upper = nick.to_uppercase();
    for (index, rank) in config.ranks.iter().enumerate() {
        if nick_upper.contains(&rank.to_uppercase()) {
            for stars in 1..=config.stars_per_rank {
                if nick_upper.contains(&format!("{stars} SAO")) {
                    let base = index.checked_mul(config.stars_per_rank as usize)?;
                    return u8::try_from(base + stars as usize).ok();
                }
            }
            let base = index.checked_mul(config.stars_per_rank as usize)?;
            return u8::try_from(base + 1).ok();
        }
    }
    None
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
        assert_eq!(parse_nickname(&config, "Bình thường"), None);
    }
}
