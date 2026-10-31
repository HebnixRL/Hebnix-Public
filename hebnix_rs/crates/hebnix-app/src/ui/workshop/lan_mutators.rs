//! Rocket League game settings for LAN hosting, as Rocket Plugin lists them
//! (`source/RocketPlugin/RLConstants.inc`, MUTATORS). Each option is a label and
//! the game tag it writes. The first option of each setting is the default and
//! writes no tag.

pub struct Setting {
    pub label: &'static str,
    pub options: &'static [(&'static str, &'static str)],
}

pub const SETTINGS: &[Setting] = &[
    Setting { label: "Free Play", options: &[("Disable Free Play", ""), ("Enable Free Play", "FreePlay")] },
    Setting {
        label: "Match Length",
        options: &[("5 Minutes", ""), ("10 Minutes", "10Minutes"), ("20 Minutes", "20Minutes"), ("Unlimited", "UnlimitedTime")],
    },
    Setting {
        label: "Max Score",
        options: &[
            ("Default", ""),
            ("1 Goal", "Max1"),
            ("3 Goals", "Max3"),
            ("5 Goals", "Max5"),
            ("7 Goals", "Max7"),
            ("Unlimited", "UnlimitedScore"),
        ],
    },
    Setting {
        label: "Overtime",
        options: &[
            ("Unlimited", ""),
            ("+5 Max, First Score", "Overtime5MinutesFirstScore"),
            ("+5 Max, Random Team", "Overtime5MinutesRandom"),
        ],
    },
    Setting { label: "Max Time Limit", options: &[("Default", ""), ("11 Minutes", "MaxTime11Minutes")] },
    Setting {
        label: "Series Length",
        options: &[("Unlimited", ""), ("3 Games", "3Games"), ("5 Games", "5Games"), ("7 Games", "7Games")],
    },
    Setting {
        label: "Game Speed",
        options: &[("Default", ""), ("Slo-mo", "SloMoGameSpeed"), ("Time Warp", "SloMoDistanceBall")],
    },
    Setting {
        label: "Ball Max Speed",
        options: &[("Default", ""), ("Slow", "SlowBall"), ("Fast", "FastBall"), ("Super Fast", "SuperFastBall")],
    },
    Setting {
        label: "Ball Type",
        options: &[
            ("Default", ""),
            ("Cube", "Ball_CubeBall"),
            ("Puck", "Ball_Puck"),
            ("BasketBall", "Ball_BasketBall"),
            ("BeachBall", "Ball_BeachBall"),
            ("Anniversary", "Ball_Anniversary"),
            ("HauntedBall", "Ball_Haunted"),
        ],
    },
    Setting {
        label: "Ball Gravity",
        options: &[
            ("Default", ""),
            ("Low", "LowGravityBall"),
            ("High", "HighGravityBall"),
            ("Super High", "SuperGravityBall"),
        ],
    },
    Setting {
        label: "Ball Physics",
        options: &[
            ("Default", ""),
            ("Light", "LightBall"),
            ("Heavy", "HeavyBall"),
            ("Super Light", "SuperLightBall"),
            ("CurveBall", "MagnusBall"),
            ("Beach Ball Curve", "MagnusBeachBall"),
        ],
    },
    Setting {
        label: "Ball Size",
        options: &[("Default", ""), ("Small", "SmallBall"), ("Medium", "MediumBall"), ("Large", "BigBall"), ("Gigantic", "GiantBall")],
    },
    Setting {
        label: "Ball Bounciness",
        options: &[("Default", ""), ("Low", "LowBounciness"), ("High", "HighBounciness"), ("Super High", "SuperBounciness")],
    },
    Setting {
        label: "Number Of Balls",
        options: &[("One", ""), ("Two", "TwoBalls"), ("Four", "FourBalls"), ("Six", "SixBalls")],
    },
    Setting {
        label: "Boosts Amount",
        options: &[
            ("Default", ""),
            ("No Boost", "NoBooster"),
            ("Unlimited", "UnlimitedBooster"),
            ("Recharge (slow)", "SlowRecharge"),
            ("Recharge (fast)", "RapidRecharge"),
        ],
    },
    Setting {
        label: "Rumble",
        options: &[
            ("None", ""),
            ("Default", "ItemsMode"),
            ("Slow", "ItemsModeSlow"),
            ("Civilized", "ItemsModeBallManipulators"),
            ("Destruction Derby", "ItemsModeCarManipulators"),
            ("Spring Loaded", "ItemsModeSprings"),
            ("Spikes Only", "ItemsModeSpikes"),
            ("Rugby", "ItemsModeRugby"),
            ("Haunted Ball Beam", "ItemsModeHauntedBallBeam"),
            ("Tactical Rumble", "ItemsModeSelection"),
        ],
    },
    Setting {
        label: "Boost Strength",
        options: &[("1X", ""), ("1.5X", "BoostMultiplier1_5x"), ("2X", "BoostMultiplier2x"), ("10X", "BoostMultiplier10x")],
    },
    Setting {
        label: "Gravity",
        options: &[
            ("Default", ""),
            ("Low", "LowGravity"),
            ("High", "HighGravity"),
            ("Super High", "SuperGravity"),
            ("Reverse", "ReverseGravity"),
        ],
    },
    Setting {
        label: "Demolish",
        options: &[
            ("Default", ""),
            ("Disabled", "NoDemolish"),
            ("Friendly Fire", "DemolishAll"),
            ("On Contact", "AlwaysDemolishOpposing"),
            ("On Contact (FF)", "AlwaysDemolish"),
        ],
    },
    Setting {
        label: "Respawn Time",
        options: &[
            ("3 Seconds", ""),
            ("2 Seconds", "TwoSecondsRespawn"),
            ("1 Second", "OneSecondsRespawn"),
            ("Disable Goal Reset", "DisableGoalDelay"),
        ],
    },
    Setting { label: "Bot Loadouts", options: &[("Default", ""), ("Random", "RandomizedBotLoadouts")] },
    Setting { label: "Audio", options: &[("Default", ""), ("Haunted", "HauntedAudio")] },
    Setting {
        label: "Game Event",
        options: &[("Default", ""), ("Haunted", "HauntedGameEventRules"), ("Rugby", "RugbyGameEventRules")],
    },
];

/// Split game tags into the chosen option of each setting, and the tags no
/// setting claims (kept so nothing typed or written by hand is lost).
pub fn parse<'a>(tags: impl Iterator<Item = &'a str>) -> (Vec<usize>, Vec<&'a str>) {
    let mut picks = vec![0; SETTINGS.len()];
    let mut extra = Vec::new();
    for tag in tags {
        let found = SETTINGS.iter().enumerate().find_map(|(i, s)| {
            s.options.iter().position(|(_, t)| !t.is_empty() && *t == tag).map(|j| (i, j))
        });
        match found {
            Some((i, j)) => picks[i] = j,
            None => extra.push(tag),
        }
    }
    (picks, extra)
}

/// The game tags for the chosen options, default options writing nothing.
pub fn tags(picks: &[usize]) -> impl Iterator<Item = &'static str> + '_ {
    SETTINGS
        .iter()
        .enumerate()
        .map(move |(i, s)| s.options[picks.get(i).copied().unwrap_or(0)].1)
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip_through_picks() {
        let (picks, extra) = parse("10Minutes,Ball_Puck,Lan,FreePlay".split(','));
        assert_eq!(extra, ["Lan"]);
        assert_eq!(tags(&picks).collect::<Vec<_>>(), ["FreePlay", "10Minutes", "Ball_Puck"]);
    }
}
