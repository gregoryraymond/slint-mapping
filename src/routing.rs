//! [`Router`] — the adapter trait every routing engine implements.
//!
//! Parallel to [`crate::TileSource`] but for "from A to B" rather than
//! "give me tile (x, y, z)". A router takes a [`RouteRequest`] (two or
//! more waypoints + a transport [`Profile`]) and returns a [`Route`]
//! containing the polyline geometry, total distance/duration, and a
//! step-by-step list of [`Maneuver`]s suitable for turn-by-turn UIs.
//!
//! The call is synchronous and may block on a network round-trip —
//! callers running in a Slint event loop should spawn a worker thread
//! and dispatch the result back via `slint::invoke_from_event_loop`.
//! Concrete implementations live in [`crate::routers`]; the default
//! HTTP impl is [`crate::routers::OsrmRouter`].
//!
//! Routing engines disagree on the exact set of maneuver categories
//! they emit. [`ManeuverKind`] is the lossy union — concrete routers
//! map their engine-native vocabulary onto this enum, falling back to
//! [`ManeuverKind::Other`] for anything that doesn't fit.

/// One routing request. At least two waypoints are required (origin +
/// destination); intermediate waypoints become "via" points the route
/// must pass through in order.
#[derive(Debug, Clone)]
pub struct RouteRequest {
    /// `(longitude, latitude)` pairs. The first is the origin, the
    /// last is the destination, anything in between is a forced via.
    pub waypoints: Vec<(f64, f64)>,
    pub profile: Profile,
}

impl RouteRequest {
    /// Two-waypoint convenience constructor.
    pub fn from_to(origin: (f64, f64), destination: (f64, f64), profile: Profile) -> Self {
        Self {
            waypoints: vec![origin, destination],
            profile,
        }
    }
}

/// Transport mode. Routers may not support every profile — those that
/// don't return [`RouteError::InvalidRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Driving,
    Cycling,
    Walking,
}

/// A computed route from origin to destination.
#[derive(Debug, Clone)]
pub struct Route {
    /// Densified polyline as `(longitude, latitude)` pairs. Render
    /// this as a connected line over the tile layer.
    pub geometry: Vec<(f64, f64)>,
    /// Total route length in metres.
    pub distance_m: f64,
    /// Total estimated time in seconds.
    pub duration_s: f64,
    /// Step-by-step maneuvers from depart to arrive, in order. Empty
    /// for engines that don't expose step instructions.
    pub maneuvers: Vec<Maneuver>,
}

/// One step in a route — a "turn left at the next junction" cue.
#[derive(Debug, Clone)]
pub struct Maneuver {
    /// Where the cue applies, `(longitude, latitude)`. For a turn,
    /// the junction itself; for a depart, the trip origin; for an
    /// arrive, the destination.
    pub location: (f64, f64),
    pub kind: ManeuverKind,
    /// Distance in metres from this maneuver's location to the *next*
    /// maneuver. Zero for the final `Arrive` step.
    pub distance_m: f64,
    /// Estimated time in seconds until the next maneuver.
    pub duration_s: f64,
    /// Street / road name the route follows from this step. `None`
    /// when the engine couldn't identify a name (private road,
    /// unnamed track, off-network segment).
    pub road_name: Option<String>,
}

impl Maneuver {
    /// Human-readable cue suitable for a list view or voice prompt.
    /// Concrete routers can override by populating `road_name` and
    /// then formatting here. Kept on `Maneuver` (not on `Router`) so
    /// the same logic applies regardless of which engine produced the
    /// step.
    pub fn instruction_text(&self) -> String {
        use ManeuverKind::*;
        let road = self
            .road_name
            .as_deref()
            .map(|n| format!(" onto {n}"))
            .unwrap_or_default();
        let road_on = self
            .road_name
            .as_deref()
            .map(|n| format!(" on {n}"))
            .unwrap_or_default();
        match self.kind {
            Depart => format!("Head out{road_on}"),
            Arrive => "Arrive at destination".to_string(),
            Continue => format!("Continue straight{road_on}"),
            TurnSlightLeft => format!("Bear left{road}"),
            TurnLeft => format!("Turn left{road}"),
            TurnSharpLeft => format!("Take a sharp left{road}"),
            TurnSlightRight => format!("Bear right{road}"),
            TurnRight => format!("Turn right{road}"),
            TurnSharpRight => format!("Take a sharp right{road}"),
            UTurn => format!("Make a U-turn{road}"),
            Merge => format!("Merge{road}"),
            Roundabout { exit } => {
                format!("At the roundabout, take exit {exit}{road}")
            }
            Exit => format!("Take the exit{road}"),
            Fork => format!("Keep at the fork{road}"),
            Other => format!("Continue{road_on}"),
        }
    }
}

/// Lossy union over the maneuver categories every routing engine
/// emits. New variants are added as engines expose meaningfully
/// different cues; unrecognised types map to [`ManeuverKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManeuverKind {
    Depart,
    Arrive,
    Continue,
    TurnSlightLeft,
    TurnLeft,
    TurnSharpLeft,
    TurnSlightRight,
    TurnRight,
    TurnSharpRight,
    UTurn,
    Merge,
    /// Exit number is 1-indexed (the first exit clockwise from entry).
    Roundabout {
        exit: u8,
    },
    Exit,
    Fork,
    Other,
}

/// Reasons a routing call can fail. `Network` and `Parse` wrap the
/// engine-specific message; `InvalidRequest` covers refusals from the
/// engine (unsupported profile, malformed waypoints, no route between
/// the given points).
#[derive(Debug)]
pub enum RouteError {
    /// Fewer than two waypoints, or a profile the engine doesn't support.
    InvalidRequest(String),
    /// HTTP / transport-level failure.
    Network(String),
    /// Engine returned a malformed or unexpected response body.
    Parse(String),
    /// Engine returned successfully but reports no route exists between
    /// the given waypoints (disconnected graph, all profiles excluded).
    NoRoute,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::InvalidRequest(msg) => write!(f, "invalid route request: {msg}"),
            RouteError::Network(msg) => write!(f, "routing network error: {msg}"),
            RouteError::Parse(msg) => write!(f, "routing parse error: {msg}"),
            RouteError::NoRoute => write!(f, "no route between waypoints"),
        }
    }
}

impl std::error::Error for RouteError {}

/// A routing engine adapter. Implement for any backend (OSRM,
/// Valhalla, GraphHopper, ORS, an offline graph) that can answer
/// "from A to B by mode M".
///
/// The call is **synchronous and may block on the network**. Callers
/// driving a Slint UI should run it on a worker thread and dispatch
/// the result back to the UI via `slint::invoke_from_event_loop`.
pub trait Router: Send + Sync {
    fn route(&self, request: &RouteRequest) -> Result<Route, RouteError>;
}

// Blanket impl so a `Box<dyn Router>` is itself a Router — same shape
// as the TileSource blanket impl, lets consumers swap routers at
// runtime without writing a forwarding shim.
impl<T: Router + ?Sized> Router for Box<T> {
    fn route(&self, request: &RouteRequest) -> Result<Route, RouteError> {
        (**self).route(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_text_uses_road_name_when_present() {
        let m = Maneuver {
            location: (0.0, 0.0),
            kind: ManeuverKind::TurnLeft,
            distance_m: 100.0,
            duration_s: 30.0,
            road_name: Some("Main Street".into()),
        };
        assert_eq!(m.instruction_text(), "Turn left onto Main Street");
    }

    #[test]
    fn instruction_text_handles_missing_road_name() {
        let m = Maneuver {
            location: (0.0, 0.0),
            kind: ManeuverKind::TurnRight,
            distance_m: 100.0,
            duration_s: 30.0,
            road_name: None,
        };
        assert_eq!(m.instruction_text(), "Turn right");
    }

    #[test]
    fn roundabout_includes_exit_number() {
        let m = Maneuver {
            location: (0.0, 0.0),
            kind: ManeuverKind::Roundabout { exit: 3 },
            distance_m: 50.0,
            duration_s: 10.0,
            road_name: Some("A1".into()),
        };
        assert!(m.instruction_text().contains("exit 3"));
        assert!(m.instruction_text().contains("A1"));
    }

    #[test]
    fn from_to_constructs_two_waypoint_request() {
        let req = RouteRequest::from_to((-0.1276, 51.5074), (-0.0876, 51.5174), Profile::Driving);
        assert_eq!(req.waypoints.len(), 2);
        assert_eq!(req.profile, Profile::Driving);
    }
}
