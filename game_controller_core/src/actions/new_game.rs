use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext};

/// This struct defines an action which starts the game over: the game state is what it was at the
/// start of the program (state initial, first half, no goals, no penalties, full clock and
/// budgets), the undo history and any delayed game state are dropped. The teams, the side mapping
/// and the kick-off side stay those of the launcher. It is meant for simulators and test tools
/// that play many games in one run, and it has no button in the UI. It is always legal.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NewGame;

impl Action for NewGame {
    fn execute(&self, c: &mut ActionContext) {
        // The stopped state is a property of the referee, not of the game, and persists (as with undo).
        let stopped = c.game.stopped;
        *c.game = crate::initial_game(c.params);
        c.game.stopped = stopped;
        c.clear_history();
        c.cancel_delay();
    }

    fn is_legal(&self, _c: &ActionContext) -> bool {
        true
    }
}
