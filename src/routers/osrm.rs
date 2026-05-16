//! [`OsrmRouter`] — `Router` impl backed by an OSRM HTTP server.
//!
//! OSRM is the de-facto Open Source Routing Machine for OSM-based
//! road networks. Two deployment modes:
//!
//! * **Demo server** at `https://router.project-osrm.org` — no API
//!   key, only `driving` profile, **prototyping only** (their TOS
//!   explicitly forbids production use). [`OsrmRouter::new`] points
//!   here.
//! * **Self-hosted** via the `osrm-backend` binary loaded with a
//!   prepared `.osrm` graph. Use [`OsrmRouter::with_base_url`] to
//!   point at your instance; you choose the supported profiles when
//!   you build the graphs.
//!
//! The HTTP call is blocking — call from a worker thread, dispatch
//! results back to the UI via `slint::invoke_from_event_loop`.
//!
//! # Example
//!
//! ```ignore
//! use slint_mapping::{routers::OsrmRouter, routing::{Profile, RouteRequest, Router}};
//!
//! let router = OsrmRouter::new();
//! let req = RouteRequest::from_to((-0.1276, 51.5074), (-0.0876, 51.5174), Profile::Driving);
//! std::thread::spawn(move || {
//!     match router.route(&req) {
//!         Ok(route) => println!("{} m in {} s", route.distance_m, route.duration_s),
//!         Err(e) => eprintln!("routing failed: {e}"),
//!     }
//! });
//! ```

use crate::routing::{
    Maneuver, ManeuverKind, Profile, Route, RouteError, RouteRequest, Router,
};
use serde::Deserialize;
use std::io::Read;
use std::sync::Mutex;

/// Public demo OSRM server. No API key required but throughput is
/// capped and only the `driving` profile is available. Production
/// deployments should self-host.
pub const OSRM_DEMO_URL: &str = "https://router.project-osrm.org";

pub struct OsrmRouter {
    base_url: String,
    user_agent: Mutex<String>,
    timeout_ms: u64,
}

impl OsrmRouter {
    /// Point at the public demo server. Suitable only for prototypes
    /// — the demo server's TOS forbids production traffic.
    pub fn new() -> Self {
        Self::with_base_url(OSRM_DEMO_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            user_agent: Mutex::new(String::from(
                "slint-mapping/0.1 (+https://github.com/slint-ui)",
            )),
            timeout_ms: 10_000,
        }
    }

    /// Override the HTTP User-Agent. Self-hosted instances may not
    /// care, but the demo server logs by UA — identify your app.
    pub fn with_user_agent(self, ua: impl Into<String>) -> Self {
        *self.user_agent.lock().unwrap() = ua.into();
        self
    }

    pub fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}

impl Default for OsrmRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Router for OsrmRouter {
    fn route(&self, request: &RouteRequest) -> Result<Route, RouteError> {
        if request.waypoints.len() < 2 {
            return Err(RouteError::InvalidRequest(
                "need at least 2 waypoints (origin + destination)".into(),
            ));
        }
        let url = build_url(&self.base_url, request);
        let ua = self.user_agent.lock().unwrap().clone();

        let resp = ureq::get(&url)
            .set("User-Agent", &ua)
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .call()
            .map_err(|e| RouteError::Network(e.to_string()))?;

        let mut body = String::new();
        resp.into_reader()
            .read_to_string(&mut body)
            .map_err(|e| RouteError::Network(format!("read body: {e}")))?;

        parse_response(&body)
    }
}

/// Map a [`Profile`] to the OSRM URL segment. Demo server only
/// implements `driving`; self-hosted instances may implement any
/// subset depending on which graphs were prepared at build time.
fn profile_str(p: Profile) -> &'static str {
    match p {
        Profile::Driving => "driving",
        Profile::Cycling => "cycling",
        Profile::Walking => "walking",
    }
}

pub(crate) fn build_url(base: &str, req: &RouteRequest) -> String {
    let coords = req
        .waypoints
        .iter()
        .map(|(lon, lat)| format!("{lon},{lat}"))
        .collect::<Vec<_>>()
        .join(";");
    // `overview=full` keeps every shape point on the geometry — without
    // it OSRM hands back a simplified outline that snaps corners. For
    // turn-by-turn we want the engine's full densification.
    format!(
        "{base}/route/v1/{}/{coords}?steps=true&geometries=geojson&overview=full",
        profile_str(req.profile),
    )
}

// ---- Response shape (serde mirror) ----
//
// Only the fields we need are deserialised. OSRM emits more (waypoint
// snap distances, leg summaries, weights) that we ignore by omission.
// Using `#[serde(default)]` on optional fields so a server returning a
// partial schema doesn't fail the parse outright.

#[derive(Deserialize)]
struct OsrmResponse {
    code: String,
    #[serde(default)]
    routes: Vec<OsrmRoute>,
}

#[derive(Deserialize)]
struct OsrmRoute {
    geometry: OsrmGeometry,
    distance: f64,
    duration: f64,
    #[serde(default)]
    legs: Vec<OsrmLeg>,
}

#[derive(Deserialize)]
struct OsrmGeometry {
    // Always a LineString for our requests; ignore `type`.
    coordinates: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
struct OsrmLeg {
    #[serde(default)]
    steps: Vec<OsrmStep>,
}

#[derive(Deserialize)]
struct OsrmStep {
    distance: f64,
    duration: f64,
    #[serde(default)]
    name: String,
    maneuver: OsrmManeuver,
}

#[derive(Deserialize)]
struct OsrmManeuver {
    location: [f64; 2],
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    modifier: String,
    #[serde(default)]
    exit: Option<u8>,
}

pub(crate) fn parse_response(body: &str) -> Result<Route, RouteError> {
    let resp: OsrmResponse = serde_json::from_str(body)
        .map_err(|e| RouteError::Parse(format!("json: {e}")))?;
    // OSRM uses a string status code: "Ok" on success, "NoRoute" /
    // "NoSegment" / "InvalidValue" / etc. on refusal.
    match resp.code.as_str() {
        "Ok" => {}
        "NoRoute" | "NoSegment" => return Err(RouteError::NoRoute),
        other => return Err(RouteError::Parse(format!("OSRM code: {other}"))),
    }
    let route = resp
        .routes
        .into_iter()
        .next()
        .ok_or_else(|| RouteError::Parse("response had no routes".into()))?;

    let geometry = route
        .geometry
        .coordinates
        .into_iter()
        .map(|[lon, lat]| (lon, lat))
        .collect();

    let mut maneuvers = Vec::new();
    for leg in route.legs {
        for step in leg.steps {
            let kind = map_maneuver(&step.maneuver.kind, &step.maneuver.modifier, step.maneuver.exit);
            let road_name = if step.name.is_empty() {
                None
            } else {
                Some(step.name)
            };
            maneuvers.push(Maneuver {
                location: (step.maneuver.location[0], step.maneuver.location[1]),
                kind,
                distance_m: step.distance,
                duration_s: step.duration,
                road_name,
            });
        }
    }

    Ok(Route {
        geometry,
        distance_m: route.distance,
        duration_s: route.duration,
        maneuvers,
    })
}

/// Map OSRM's (`type`, `modifier`, `exit`) triple onto a
/// [`ManeuverKind`]. Unknown combinations degrade to
/// [`ManeuverKind::Other`] rather than failing the parse — better to
/// show a generic cue than drop the step.
fn map_maneuver(kind: &str, modifier: &str, exit: Option<u8>) -> ManeuverKind {
    match kind {
        "depart" => ManeuverKind::Depart,
        "arrive" => ManeuverKind::Arrive,
        "continue" | "new name" => ManeuverKind::Continue,
        "merge" => ManeuverKind::Merge,
        "fork" => ManeuverKind::Fork,
        // OSRM uses "exit roundabout" / "exit rotary" for the second
        // "you're now leaving the roundabout" step, and "roundabout" /
        // "rotary" for the first "enter the roundabout, take exit N"
        // step. Collapse both rotary variants onto the roundabout kind.
        "roundabout" | "rotary" => {
            ManeuverKind::Roundabout { exit: exit.unwrap_or(1) }
        }
        "exit roundabout" | "exit rotary" => ManeuverKind::Exit,
        "off ramp" | "on ramp" => ManeuverKind::Exit,
        "turn" => match modifier {
            "slight left" => ManeuverKind::TurnSlightLeft,
            "left" => ManeuverKind::TurnLeft,
            "sharp left" => ManeuverKind::TurnSharpLeft,
            "slight right" => ManeuverKind::TurnSlightRight,
            "right" => ManeuverKind::TurnRight,
            "sharp right" => ManeuverKind::TurnSharpRight,
            "uturn" => ManeuverKind::UTurn,
            "straight" => ManeuverKind::Continue,
            _ => ManeuverKind::Other,
        },
        _ => ManeuverKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_url_encodes_two_waypoints() {
        let req = RouteRequest::from_to((-0.1276, 51.5074), (-0.0876, 51.5174), Profile::Driving);
        let url = build_url("https://example.com", &req);
        assert_eq!(
            url,
            "https://example.com/route/v1/driving/-0.1276,51.5074;-0.0876,51.5174\
             ?steps=true&geometries=geojson&overview=full"
        );
    }

    #[test]
    fn build_url_handles_via_waypoints() {
        let req = RouteRequest {
            waypoints: vec![(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)],
            profile: Profile::Cycling,
        };
        let url = build_url("https://example.com", &req);
        assert!(url.contains("/cycling/0,0;1,1;2,2?"));
    }

    /// Minimal hand-built OSRM-shaped response used to exercise the
    /// parser without hitting the network. Covers: success code, a
    /// LineString geometry, two legs with depart/turn/arrive steps,
    /// missing road name, roundabout with `exit`.
    const SAMPLE_OK_BODY: &str = r#"{
        "code": "Ok",
        "routes": [{
            "geometry": {
                "type": "LineString",
                "coordinates": [[-0.1, 51.5], [-0.05, 51.51], [0.0, 51.52]]
            },
            "distance": 1234.5,
            "duration": 567.8,
            "legs": [
                {
                    "steps": [
                        {
                            "distance": 50.0, "duration": 10.0, "name": "Origin St",
                            "maneuver": {"location": [-0.1, 51.5], "type": "depart", "modifier": ""}
                        },
                        {
                            "distance": 1000.0, "duration": 400.0, "name": "Main St",
                            "maneuver": {"location": [-0.05, 51.51], "type": "turn", "modifier": "left"}
                        }
                    ]
                },
                {
                    "steps": [
                        {
                            "distance": 184.5, "duration": 90.0, "name": "",
                            "maneuver": {"location": [-0.02, 51.515], "type": "roundabout", "modifier": "right", "exit": 3}
                        },
                        {
                            "distance": 0.0, "duration": 0.0, "name": "Destination Ave",
                            "maneuver": {"location": [0.0, 51.52], "type": "arrive", "modifier": ""}
                        }
                    ]
                }
            ]
        }]
    }"#;

    #[test]
    fn parses_ok_response() {
        let route = parse_response(SAMPLE_OK_BODY).expect("parse");
        assert_eq!(route.distance_m, 1234.5);
        assert_eq!(route.duration_s, 567.8);
        assert_eq!(route.geometry.len(), 3);
        assert_eq!(route.geometry[0], (-0.1, 51.5));
        assert_eq!(route.maneuvers.len(), 4);

        assert_eq!(route.maneuvers[0].kind, ManeuverKind::Depart);
        assert_eq!(route.maneuvers[0].road_name.as_deref(), Some("Origin St"));

        assert_eq!(route.maneuvers[1].kind, ManeuverKind::TurnLeft);

        match route.maneuvers[2].kind {
            ManeuverKind::Roundabout { exit } => assert_eq!(exit, 3),
            other => panic!("expected Roundabout, got {other:?}"),
        }
        // Empty `name` in JSON should land as None on the parsed side.
        assert!(route.maneuvers[2].road_name.is_none());

        assert_eq!(route.maneuvers[3].kind, ManeuverKind::Arrive);
    }

    #[test]
    fn no_route_status_maps_to_error() {
        let body = r#"{"code":"NoRoute","routes":[]}"#;
        match parse_response(body) {
            Err(RouteError::NoRoute) => {}
            other => panic!("expected NoRoute, got {other:?}"),
        }
    }

    #[test]
    fn unknown_status_is_parse_error() {
        let body = r#"{"code":"InvalidOptions","routes":[]}"#;
        match parse_response(body) {
            Err(RouteError::Parse(msg)) => assert!(msg.contains("InvalidOptions")),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn map_maneuver_covers_every_turn_modifier() {
        // Spot-check the turn matrix — a regression in this table
        // silently degrades instructions to "Continue" which is bad UX
        // but not crash-y, so explicit coverage matters.
        assert_eq!(map_maneuver("turn", "left", None), ManeuverKind::TurnLeft);
        assert_eq!(map_maneuver("turn", "sharp right", None), ManeuverKind::TurnSharpRight);
        assert_eq!(map_maneuver("turn", "uturn", None), ManeuverKind::UTurn);
        assert_eq!(map_maneuver("turn", "straight", None), ManeuverKind::Continue);
        // Unknown modifier on a turn falls through to Other rather than
        // misclassifying as Continue.
        assert_eq!(map_maneuver("turn", "wibble", None), ManeuverKind::Other);
    }

    #[test]
    fn roundabout_without_exit_defaults_to_one() {
        // OSRM sometimes omits `exit` on legacy graphs — defaulting to
        // 1 keeps the instruction renderable rather than crashing.
        let kind = map_maneuver("roundabout", "right", None);
        assert_eq!(kind, ManeuverKind::Roundabout { exit: 1 });
    }
}
