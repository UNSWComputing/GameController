//! `newGame` brings a played game back to exactly the state of a new one.

use std::time::Duration;

use enum_map::enum_map;
use game_controller_core::{
    action::VAction,
    actions::{FreeSetPlay, Goal, NewGame, Penalize, StartSetPlay, WaitForSetPlay},
    initial_game,
    log::NullLogger,
    types::{
        ActionSource, Color, CompetitionParams, GameParams, Params, PenaltyCall, PlayerNumber,
        SetPlay, Side, SideMapping, State, TeamParams, TestParams,
    },
    GameController,
};

fn params() -> Params {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../config/middle_advanced/params.yaml");
    let competition: CompetitionParams =
        serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    Params {
        competition,
        game: GameParams {
            teams: enum_map! {
                Side::Home => TeamParams { number: 1, field_player_color: Color::Blue, goalkeeper_color: Color::Red },
                Side::Away => TeamParams { number: 18, field_player_color: Color::Black, goalkeeper_color: Color::Green },
            },
            kick_off_side: Side::Home,
            side_mapping: SideMapping::HomeDefendsLeftGoal,
            test: TestParams::default(),
        },
    }
}

#[test]
fn a_played_game_starts_over() {
    let mut gc = GameController::new(params(), Box::new(NullLogger));
    let fresh = gc.get_game(false).clone();
    assert_eq!(fresh, initial_game(&params()));

    let start = VAction::StartSetPlay(StartSetPlay { side: Some(Side::Home), set_play: SetPlay::KickOff });
    assert!(gc.apply(start, ActionSource::User));
    assert_eq!(gc.get_game(false).state, State::Ready);
    gc.seek(Duration::from_secs(20));
    assert!(gc.apply(VAction::WaitForSetPlay(WaitForSetPlay), ActionSource::User));
    assert!(gc.apply(VAction::FreeSetPlay(FreeSetPlay), ActionSource::User));
    assert_eq!(gc.get_game(false).state, State::Playing);
    let penalize = VAction::Penalize(Penalize {
        side: Side::Away,
        player: PlayerNumber::new(2),
        call: PenaltyCall::Pushing,
    });
    assert!(gc.apply(penalize, ActionSource::User));
    assert!(gc.apply(VAction::Goal(Goal { side: Side::Home }), ActionSource::User));
    assert_ne!(*gc.get_game(false), fresh);

    assert!(gc.apply(VAction::NewGame(NewGame), ActionSource::User));
    assert_eq!(*gc.get_game(false), fresh);
    assert_eq!(gc.get_game(false).teams[Side::Home].score, 0);
    assert!(gc.get_undo_actions(10).is_empty());
}
