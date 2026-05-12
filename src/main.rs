use axum::{routing::get, Router, extract::State};
use chrono::{Timelike, TimeZone};
use gtfs_structures::Gtfs;
use gtfs_realtime::*;
use gtfs_realtime::vehicle_position::*;
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

fn get_active_trip_id<'a>(route_id: &str, gtfs: &'a Gtfs) -> Option<&'a String> {
    let gtfs_route_id = format!("TL-{}", route_id);
    let now = chrono::Utc::now().with_timezone(&chrono_tz::US::Pacific);
    let seconds_past_midnight = now.hour() * 3600 + now.minute() * 60 + now.second();

    let mut fallback = None;
    for trip in gtfs.trips.values().filter(|t| t.route_id == gtfs_route_id) {
        if fallback.is_none() {
            fallback = Some(&trip.id);
        }
        for freq in &trip.frequencies {
            if seconds_past_midnight >= freq.start_time && seconds_past_midnight <= freq.end_time {
                return Some(&trip.id);
            }
        }
    }
    fallback
}

async fn update_feeds(state: Arc<AppState>) {
    loop {
        // Fetch new json
        match reqwest::get("https://ucirvine.transloc.com/Services/JSONPRelay.svc/GetMapVehiclePoints?_=1712182850877").await {
            Ok(res) => {
                if let Ok(text) = res.text().await {
                    if let Ok(data) = serde_json::from_str::<Vec<AnteaterExpressData>>(&text) {
                        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                        
                        let mut positions = vec![];
                        let mut trip_updates = vec![];

                        let mut history_write = state.history.write().await;

                        for vehicle in data {
                            let trip_id = get_active_trip_id(&vehicle.RouteID.to_string(), &state.gtfs)
                                .cloned();
                            
                            let start_time = if let Some(ref t_id) = trip_id {
                                state.gtfs.trips.get(t_id).and_then(|t| t.frequencies.first().map(|f| f.start_time))
                            } else {
                                None
                            };

                            let trip_desc = TripDescriptor {
                                trip_id: trip_id.clone(),
                                route_id: Some(format!("TL-{}", vehicle.RouteID)),
                                direction_id: Some(0),
                                start_time: start_time.map(|s| format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)),
                                start_date: None,
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

                            let v_hist = history_write.entry(vehicle.VehicleID).or_insert(VehicleHistory {
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
                                let dist = (dx*dx + dy*dy).sqrt() * 111000.0; // approx meters
                                let speed = dist / dt;
                                // Assume expected speed is ~5 m/s. If slower, add to delay.
                                if dt > 0.0 {
                                    let expected_dist = 5.0 * dt;
                                    let lost_time = (expected_dist - dist) / 5.0;
                                    if lost_time > 0.0 {
                                        v_hist.current_delay_secs += lost_time as i32;
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

                            if trip_id.is_some() {
                                trip_updates.push(FeedEntity {
                                    id: format!("tu_{}", vehicle.VehicleID),
                                    is_deleted: Some(false),
                                    trip_update: Some(TripUpdate {
                                        trip: trip_desc,
                                        vehicle: Some(vehicle_desc),
                                        stop_time_update: vec![],
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
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
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
