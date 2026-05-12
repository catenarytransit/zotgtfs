use axum::{Router, extract::State, routing::get};
use chrono::Timelike;
use gtfs_realtime::vehicle_position::*;
use gtfs_realtime::*;
use gtfs_structures::Gtfs;
use prost::Message;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

struct VehicleHistory {
    positions: Vec<(u64, Position)>,
    current_delay_secs: i32,
}

// State shared across axum workers and background updater
struct AppState {
    gtfs: Gtfs,
    positions_feed: RwLock<Vec<u8>>,
    trip_updates_feed: RwLock<Vec<u8>>,
    history: RwLock<HashMap<i16, VehicleHistory>>,
}

#[derive(Deserialize, Debug)]
#[allow(non_snake_case)]
struct AnteaterExpressData {
    GroundSpeed: f32,
    Heading: f32,
    Latitude: f32,
    Longitude: f32,
    Name: String,
    RouteID: i32,
    VehicleID: i16,
}

#[derive(Clone, Debug)]
struct ActiveTrip {
    trip_id: String,
    route_id: String,
    start_time: Option<u32>,
}

fn format_gtfs_time(seconds: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

fn get_active_trip(route_id: i32, gtfs: &Gtfs) -> Option<ActiveTrip> {
    // TransLoc vehicle RouteID is the same numeric id used by the current UCI GTFS.
    // Older/current data can vary, so keep TL-* as a fallback instead of making it
    // the only possible match.
    let raw_route_id = route_id.to_string();
    let prefixed_route_id = format!("TL-{}", raw_route_id);

    let now = chrono::Utc::now().with_timezone(&chrono_tz::America::Los_Angeles);
    let seconds_past_midnight = now.hour() * 3600 + now.minute() * 60 + now.second();

    let mut fallback: Option<ActiveTrip> = None;

    for trip in gtfs
        .trips
        .values()
        .filter(|t| t.route_id == raw_route_id || t.route_id == prefixed_route_id)
    {
        if fallback.is_none() {
            fallback = Some(ActiveTrip {
                trip_id: trip.id.clone(),
                route_id: trip.route_id.clone(),
                start_time: trip.frequencies.first().map(|f| f.start_time).or_else(|| {
                    trip.stop_times
                        .first()
                        .and_then(|st| st.departure_time.or(st.arrival_time))
                }),
            });
        }

        for freq in &trip.frequencies {
            if seconds_past_midnight >= freq.start_time && seconds_past_midnight <= freq.end_time {
                let instance_start = if freq.headway_secs > 0 {
                    freq.start_time
                        + ((seconds_past_midnight - freq.start_time) / freq.headway_secs)
                            * freq.headway_secs
                } else {
                    freq.start_time
                };

                return Some(ActiveTrip {
                    trip_id: trip.id.clone(),
                    route_id: trip.route_id.clone(),
                    start_time: Some(instance_start),
                });
            }
        }

        let trip_start = trip
            .stop_times
            .first()
            .and_then(|st| st.departure_time.or(st.arrival_time));
        let trip_end = trip
            .stop_times
            .last()
            .and_then(|st| st.arrival_time.or(st.departure_time));

        if let (Some(start), Some(end)) = (trip_start, trip_end) {
            if seconds_past_midnight >= start && seconds_past_midnight <= end {
                return Some(ActiveTrip {
                    trip_id: trip.id.clone(),
                    route_id: trip.route_id.clone(),
                    start_time: Some(start),
                });
            }
        }
    }

    fallback
}

async fn update_feeds(state: Arc<AppState>) {
    loop {
        // Fetch new json
        match reqwest::get(
            "https://ucirvine.transloc.com/Services/JSONPRelay.svc/GetMapVehiclePoints",
        )
        .await
        {
            Ok(res) => {
                if let Ok(text) = res.text().await {
                    if let Ok(data) = serde_json::from_str::<Vec<AnteaterExpressData>>(&text) {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs();

                        let mut positions = vec![];
                        let mut trip_updates = vec![];

                        let mut history_write = state.history.write().await;

                        for vehicle in data {
                            let active_trip = get_active_trip(vehicle.RouteID, &state.gtfs);
                            let trip_id = active_trip.as_ref().map(|t| t.trip_id.clone());
                            let gtfs_route_id = active_trip
                                .as_ref()
                                .map(|t| t.route_id.clone())
                                .unwrap_or_else(|| vehicle.RouteID.to_string());
                            let start_time = active_trip.as_ref().and_then(|t| t.start_time);

                            let now_la =
                                chrono::Utc::now().with_timezone(&chrono_tz::America::Los_Angeles);
                            let start_date_str = now_la.format("%Y%m%d").to_string();

                            let trip_desc = TripDescriptor {
                                trip_id: trip_id.clone(),
                                route_id: Some(gtfs_route_id),
                                direction_id: Some(0),
                                start_time: start_time.map(format_gtfs_time),
                                start_date: Some(start_date_str),
                                schedule_relationship: None,
                                modified_trip: None,
                            };

                            let vehicle_desc = VehicleDescriptor {
                                id: Some(vehicle.VehicleID.to_string()),
                                label: Some(vehicle.Name.clone()),
                                license_plate: None,
                                wheelchair_accessible: None,
                            };

                            let current_pos = Position {
                                latitude: vehicle.Latitude,
                                longitude: vehicle.Longitude,
                                bearing: Some(vehicle.Heading),
                                odometer: None,
                                speed: Some(vehicle.GroundSpeed * (1.0 / 3.6)),
                            };

                            let v_hist =
                                history_write
                                    .entry(vehicle.VehicleID)
                                    .or_insert(VehicleHistory {
                                        positions: vec![],
                                        current_delay_secs: 0,
                                    });
                            v_hist.positions.push((now, current_pos.clone()));
                            if v_hist.positions.len() > 100 {
                                v_hist.positions.remove(0); // keep history bounded
                            }

                            // Calculate delay trivially: if it's moving slower than average GTFS speed, accumulate delay
                            // (A real alg would snap shape. For now we use a heuristic as requested)
                            if v_hist.positions.len() > 1 {
                                let last = &v_hist.positions[v_hist.positions.len() - 2];
                                let dt = (now - last.0) as f32;
                                // distance in coords approx using pythagoras for a tiny diff
                                let dx = current_pos.longitude - last.1.longitude;
                                let dy = current_pos.latitude - last.1.latitude;
                                let dist = (dx * dx + dy * dy).sqrt() * 111000.0; // approx meters
                                // Assume expected speed is ~5 m/s. If slower, add to delay.
                                if dt > 0.0 {
                                    let expected_dist = 5.0 * dt;
                                    let lost_time = (expected_dist - dist) / 5.0;
                                    if lost_time > 0.0 {
                                        v_hist.current_delay_secs += lost_time as i32;
                                    }
                                }
                            }

                            let mut stop_time_updates = vec![];
                            if let Some(ref active) = active_trip {
                                if let Some(trip) = state.gtfs.trips.get(&active.trip_id) {
                                    use gtfs_realtime::trip_update::StopTimeEvent;
                                    use gtfs_realtime::trip_update::StopTimeUpdate;
                                    for st in &trip.stop_times {
                                        let seq = st.stop_sequence as u32;
                                        // Simple algorithm: predict based on accumulated delay
                                        let mut arrival = None;
                                        let mut departure = None;
                                        if st.arrival_time.is_some() {
                                            arrival = Some(StopTimeEvent {
                                                delay: Some(v_hist.current_delay_secs),
                                                time: None,
                                                uncertainty: None,
                                            });
                                        }
                                        if st.departure_time.is_some() {
                                            departure = Some(StopTimeEvent {
                                                delay: Some(v_hist.current_delay_secs),
                                                time: None,
                                                uncertainty: None,
                                            });
                                        }

                                        stop_time_updates.push(StopTimeUpdate {
                                            stop_sequence: Some(seq),
                                            stop_id: Some(st.stop.id.clone()),
                                            arrival,
                                            departure,
                                            departure_occupancy_status: None,
                                            schedule_relationship: None,
                                            stop_time_properties: None,
                                        });
                                    }
                                }
                            }

                            positions.push(FeedEntity {
                                id: format!("pos_{}", vehicle.VehicleID),
                                is_deleted: Some(false),
                                trip_update: None,
                                vehicle: Some(VehiclePosition {
                                    trip: Some(trip_desc.clone()),
                                    vehicle: Some(vehicle_desc.clone()),
                                    position: Some(current_pos),
                                    current_stop_sequence: None,
                                    stop_id: None,
                                    current_status: None,
                                    timestamp: Some(now),
                                    congestion_level: None,
                                    occupancy_status: None,
                                    occupancy_percentage: None,
                                    multi_carriage_details: vec![],
                                }),
                                alert: None,
                                shape: None,
                                stop: None,
                                trip_modifications: None,
                            });

                            if active_trip.is_some() {
                                trip_updates.push(FeedEntity {
                                    id: format!("tu_{}", vehicle.VehicleID),
                                    is_deleted: Some(false),
                                    trip_update: Some(TripUpdate {
                                        trip: trip_desc,
                                        vehicle: Some(vehicle_desc),
                                        stop_time_update: stop_time_updates,
                                        timestamp: Some(now),
                                        delay: Some(v_hist.current_delay_secs),
                                        trip_properties: None,
                                    }),
                                    vehicle: None,
                                    alert: None,
                                    shape: None,
                                    stop: None,
                                    trip_modifications: None,
                                });
                            }
                        }

                        let header = FeedHeader {
                            gtfs_realtime_version: String::from("2.0"),
                            incrementality: None,
                            timestamp: Some(now),
                        };

                        let pos_feed = FeedMessage {
                            header: header.clone(),
                            entity: positions,
                        };
                        let t_updates_feed = FeedMessage {
                            header,
                            entity: trip_updates,
                        };

                        *state.positions_feed.write().await = pos_feed.encode_to_vec();
                        *state.trip_updates_feed.write().await = t_updates_feed.encode_to_vec();
                    }
                }
            }
            Err(e) => {
                eprintln!("Error fetching bus data: {}", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn handle_vehicle_positions(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let bytes = state.positions_feed.read().await.clone();
    axum::response::Response::builder()
        .header("Content-Type", "application/octet-stream")
        .body(axum::body::Body::from(bytes))
        .unwrap()
}

async fn handle_trip_updates(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let bytes = state.trip_updates_feed.read().await.clone();
    axum::response::Response::builder()
        .header("Content-Type", "application/octet-stream")
        .body(axum::body::Body::from(bytes))
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gtfs_url = "https://ucirvine.transloc.com/Secure/Admin/Reports/GTFSDownload.aspx";
    let gtfs = Gtfs::from_url_async(gtfs_url).await.unwrap_or_default();

    let state = Arc::new(AppState {
        gtfs,
        positions_feed: RwLock::new(vec![]),
        trip_updates_feed: RwLock::new(vec![]),
        history: RwLock::new(HashMap::new()),
    });

    let state_clone = state.clone();
    tokio::spawn(async move {
        update_feeds(state_clone).await;
    });

    let app = Router::new()
        .route("/vehicle_positions", get(handle_vehicle_positions))
        .route("/trip_updates", get(handle_trip_updates))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
