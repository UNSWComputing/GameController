//! This module lets another program on the same machine apply actions, as an alternative to the
//! buttons of the UI. It is meant for simulators and test tools, and is only active if a port is
//! given with `--remote-port`.
//!
//! The receiver listens on UDP port `port` of `127.0.0.1`, so only processes on the same host can
//! reach it. Each datagram is one action in the same JSON format that the UI uses, for example
//!
//! ```json
//! {"type": "startSetPlay", "args": {"side": "home", "setPlay": "kickOff"}}
//! ```
//!
//! Actions without arguments may omit `args`. Wherever an action names a `side` (`"home"` or `"away"`),
//! `"team": <number>` may be given instead, with the team number of one of the two playing teams; the
//! GameController looks up the side. This way the sender does not need to know which team the
//! launcher put on which side. The action is queued exactly like an action from the
//! UI (it is applied only if it is legal in the current state, and it is logged and can be
//! undone). The sender gets one datagram back: `{"queued": true}` once the action was queued, or
//! `{"error": "..."}` if the request was not a valid action. Whether a queued action was legal
//! can be seen in the control messages or in the UI.

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::{net::UdpSocket, sync::mpsc};

use game_controller_core::{action::VAction, types::GameParams};

/// The maximum size of a request in bytes. Longer datagrams are rejected.
const MAX_REQUEST_SIZE: usize = 4096;

/// This struct receives actions on a loopback UDP port and queues them for the runtime.
pub struct RemoteActionReceiver {
    /// The socket on which requests arrive.
    socket: UdpSocket,
    /// The sender through which actions reach the event loop (the same one the UI uses).
    action_sender: mpsc::UnboundedSender<VAction>,
    /// The parameters of the game, to find the side of a team that is given by its number.
    game: GameParams,
}

impl RemoteActionReceiver {
    /// This function creates a receiver bound to `127.0.0.1:port` (port 0 chooses a free one).
    pub async fn new(
        port: u16,
        action_sender: mpsc::UnboundedSender<VAction>,
        game: GameParams,
    ) -> Result<Self> {
        let socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .await
            .with_context(|| format!("could not bind 127.0.0.1:{port} for remote actions"))?;
        Ok(Self {
            socket,
            action_sender,
            game,
        })
    }

    /// This function returns the address on which the receiver listens.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    /// This function receives requests forever. Errors in single requests are reported to the
    /// sender and do not stop the receiver.
    pub async fn run(self) {
        // One byte more than allowed, so that too long requests can be told apart.
        let mut buffer = vec![0u8; MAX_REQUEST_SIZE + 1];
        loop {
            let Ok((length, sender)) = self.socket.recv_from(&mut buffer).await else {
                continue;
            };
            let reply = match parse_request(&buffer[..length], &self.game) {
                Ok(action) => {
                    if self.action_sender.send(action).is_err() {
                        // The event loop is gone: the runtime is shutting down.
                        return;
                    }
                    json!({ "queued": true })
                }
                Err(error) => json!({ "error": error }),
            };
            let _ = self
                .socket
                .send_to(reply.to_string().as_bytes(), sender)
                .await;
        }
    }
}

/// This function parses a request into an action. The error is a message for the sender.
pub fn parse_request(data: &[u8], game: &GameParams) -> Result<VAction, String> {
    if data.len() > MAX_REQUEST_SIZE {
        return Err(format!("request is longer than {MAX_REQUEST_SIZE} bytes"));
    }
    let mut value: Value = serde_json::from_slice(data).map_err(|error| error.to_string())?;
    // Actions without arguments are unit structs, which are serialized as `args: null`.
    if let Some(object) = value.as_object_mut() {
        object.entry("args").or_insert(Value::Null);
        // A team number may replace the side.
        if let Some(args) = object.get_mut("args").and_then(Value::as_object_mut) {
            if let Some(team) = args.remove("team") {
                if args.contains_key("side") {
                    return Err("give either \"side\" or \"team\", not both".to_string());
                }
                let number = team
                    .as_u64()
                    .and_then(|number| u8::try_from(number).ok())
                    .ok_or_else(|| "\"team\" must be a team number".to_string())?;
                let side = game
                    .get_side(number)
                    .ok_or_else(|| format!("team {number} is not one of the playing teams"))?;
                args.insert(
                    "side".to_string(),
                    serde_json::to_value(side).map_err(|error| error.to_string())?,
                );
            }
        }
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use enum_map::enum_map;
    use game_controller_core::{
        actions::{Penalize, StartSetPlay},
        types::{
            Color, PenaltyCall, PlayerNumber, SetPlay, Side, SideMapping, TeamParams, TestParams,
        },
    };
    use std::time::Duration;
    use tokio::time::timeout;

    /// A game in which team 18 is the away team and team 1 the home team.
    fn game() -> GameParams {
        GameParams {
            teams: enum_map! {
                Side::Home => TeamParams { number: 1, field_player_color: Color::Blue, goalkeeper_color: Color::Red },
                Side::Away => TeamParams { number: 18, field_player_color: Color::Black, goalkeeper_color: Color::Green },
            },
            kick_off_side: Side::Home,
            side_mapping: SideMapping::HomeDefendsLeftGoal,
            test: TestParams::default(),
        }
    }

    fn parse_request_g(data: &[u8]) -> Result<VAction, String> {
        parse_request(data, &game())
    }

    #[test]
    fn team_numbers_replace_the_side() {
        // Team 18 is the away team in this game, whatever the sender assumes.
        let by_team = parse_request_g(
            br#"{"type":"penalize","args":{"team":18,"player":2,"call":"pushing"}}"#,
        );
        let by_side = parse_request_g(
            br#"{"type":"penalize","args":{"side":"away","player":2,"call":"pushing"}}"#,
        );
        assert_eq!(by_team, by_side);
        assert!(matches!(
            by_team,
            Ok(VAction::Penalize(Penalize { side: Side::Away, player, call: PenaltyCall::Pushing }))
                if player == PlayerNumber::new(2)
        ));
        assert_eq!(
            parse_request_g(br#"{"type":"startSetPlay","args":{"team":1,"setPlay":"kickOff"}}"#),
            Ok(VAction::StartSetPlay(StartSetPlay {
                side: Some(Side::Home),
                set_play: SetPlay::KickOff
            }))
        );
        assert!(parse_request_g(br#"{"type":"goal","args":{"team":7}}"#)
            .unwrap_err()
            .contains("not one of the playing teams"));
        assert!(parse_request_g(br#"{"type":"goal","args":{"team":"eighteen"}}"#).is_err());
        assert!(parse_request_g(br#"{"type":"goal","args":{"team":1,"side":"home"}}"#).is_err());
    }

    #[test]
    fn parses_the_actions_of_the_ui() {
        assert_eq!(
            parse_request_g(
                br#"{"type":"startSetPlay","args":{"side":"home","setPlay":"kickOff"}}"#
            ),
            Ok(VAction::StartSetPlay(StartSetPlay {
                side: Some(Side::Home),
                set_play: SetPlay::KickOff,
            }))
        );
        assert!(matches!(
            parse_request_g(br#"{"type":"goal","args":{"side":"away"}}"#),
            Ok(VAction::Goal(_))
        ));
        assert!(matches!(
            parse_request_g(
                br#"{"type":"penalize","args":{"side":"home","player":2,"call":"pushing"}}"#
            ),
            Ok(VAction::Penalize(_))
        ));
    }

    #[test]
    fn actions_without_arguments_may_omit_args() {
        assert!(matches!(
            parse_request_g(br#"{"type":"switchHalf"}"#),
            Ok(VAction::SwitchHalf(_))
        ));
        assert!(matches!(
            parse_request_g(br#"{"type":"switchHalf","args":null}"#),
            Ok(VAction::SwitchHalf(_))
        ));
    }

    #[test]
    fn rejects_bad_requests() {
        assert!(parse_request_g(b"not json").is_err());
        assert!(parse_request_g(br#"{"type":"doesNotExist","args":null}"#).is_err());
        assert!(parse_request_g(br#"{"type":"goal","args":{"side":"sideways"}}"#).is_err());
        assert!(parse_request_g(&vec![b' '; MAX_REQUEST_SIZE + 1]).is_err());
    }

    #[tokio::test]
    async fn queues_actions_and_answers() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let remote = RemoteActionReceiver::new(0, sender, game()).await.unwrap();
        let address = remote.local_addr().unwrap();
        assert!(address.ip().is_loopback());
        tokio::spawn(remote.run());

        let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut reply = [0u8; 256];

        client
            .send_to(
                br#"{"type":"startSetPlay","args":{"side":"home","setPlay":"kickOff"}}"#,
                address,
            )
            .await
            .unwrap();
        let length = timeout(Duration::from_secs(2), client.recv(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&reply[..length], br#"{"queued":true}"#);
        let action = timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(action, VAction::StartSetPlay(_)));

        client.send_to(b"garbage", address).await.unwrap();
        let length = timeout(Duration::from_secs(2), client.recv(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert!(std::str::from_utf8(&reply[..length])
            .unwrap()
            .contains("error"));
        assert!(receiver.try_recv().is_err());
    }
}
