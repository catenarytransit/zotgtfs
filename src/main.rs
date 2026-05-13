use axum::{Router, extract::State, routing::get};
use chrono::Timelike;
use gtfs_realtime::vehicle_position::*;
use gtfs_realtime::*;
use gtfs_structures::{Gtfs, StopTime, Trip};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const HISTORY_PATH_ENV: &str = "VEHICLE_HISTORY_PATH";
const DEFAULT_HISTORY_PATH: &str = "vehicle_history.json";
const HISTORY_SAVE_INTERVAL_SECS: u64 = 5;
const HISTORY_MAX_AGE_SECS: u64 = 2 * 60 * 60;
const HISTORY_MAX_SAMPLES: usize = 480;
const HISTORY_MIN_SAMPLE_INTERVAL_SECS: u64 = 10;
const HISTORY_MIN_SAMPLE_DISTANCE_M: f64 = 25.0;
const MATCH_SAMPLE_MAX_AGE_SECS: u64 = 60 * 60;
const MATCH_SAMPLE_LIMIT: usize = 36;
const MAX_CANDIDATES_PER_VEHICLE: usize = 20;
const TRIP_MATCH_WINDOW_BEFORE_SECS: i32 = 20 * 60;
const TRIP_MATCH_WINDOW_AFTER_SECS: i32 = 30 * 60;
const MAX_REASONABLE_DELAY_SECS: i32 = 30 * 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PositionSample {
    timestamp: u64,
    latitude: f32,
    longitude: f32,
    speed_mps: Option<f32>,
    heading: Option<f32>,
    transloc_route_id: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct VehicleHistory {
    positions: Vec<PositionSample>,
    current_delay_secs: i32,
    assigned_trip_id: Option<String>,
    assigned_route_id: Option<String>,
    assigned_start_time: Option<u32>,
    matched_stop_sequence: Option<u32>,
    matched_stop_id: Option<String>,
    last_transloc_route_id: Option<i32>,
    last_seen: u64,
}

impl Default for VehicleHistory {
    fn default() -> Self {
        Self {
            positions: vec![],
            current_delay_secs: 0,
            assigned_trip_id: None,
            assigned_route_id: None,
            assigned_start_time: None,
            matched_stop_sequence: None,
            matched_stop_id: None,
            last_transloc_route_id: None,
            last_seen: 0,
        }
    }
}

// State shared across axum workers and background updater.
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
struct CandidateTrip {
    trip_id: String,
    route_id: String,
    start_time: u32,
    schedule_start_time: u32,
}

#[derive(Clone, Debug)]
struct TripMatch {
    trip_id: String,
    route_id: String,
    start_time: u32,
    schedule_start_time: u32,
    score: f64,
    delay_secs: i32,
    current_stop_sequence: Option<u32>,
    stop_id: Option<String>,
}

impl TripMatch {
    fn instance_key(&self) -> String {
        format!("{}|{}|{}", self.route_id, self.trip_id, self.start_time)
    }
}

#[derive(Clone, Debug)]
struct StopMatch {
    stop_sequence: u32,
    stop_id: String,
    distance_m: f64,
    scheduled_secs: i32,
    observed_secs: i32,
    score: f64,
}

fn format_gtfs_time(seconds: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

fn history_path() -> String {
    std::env::var(HISTORY_PATH_ENV).unwrap_or_else(|_| DEFAULT_HISTORY_PATH.to_string())
}

async fn load_history_from_disk() -> HashMap<i16, VehicleHistory> {
    let path = history_path();
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => match serde_json::from_str::<HashMap<i16, VehicleHistory>>(&contents) {
            Ok(history) => history,
            Err(err) => {
                eprintln!("Ignoring invalid vehicle history file {}: {}", path, err);
                HashMap::new()
            }
        },
        Err(err) if err.kind() == ErrorKind::NotFound => HashMap::new(),
        Err(err) => {
            eprintln!("Could not read vehicle history file {}: {}", path, err);
            HashMap::new()
        }
    }
}

async fn save_history_to_disk(history: &HashMap<i16, VehicleHistory>) {
    let path = history_path();
    let path_obj = Path::new(&path);

    if let Some(parent) = path_obj.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                eprintln!(
                    "Could not create vehicle history directory {:?}: {}",
                    parent, err
                );
                return;
            }
        }
    }

    let tmp_path = format!("{}.tmp", path);
    let serialized = match serde_json::to_vec(history) {
        Ok(serialized) => serialized,
        Err(err) => {
            eprintln!("Could not serialize vehicle history: {}", err);
            return;
        }
    };

    if let Err(err) = tokio::fs::write(&tmp_path, serialized).await {
        eprintln!("Could not write vehicle history file {}: {}", tmp_path, err);
        return;
    }

    if let Err(err) = tokio::fs::rename(&tmp_path, &path).await {
        eprintln!("Could not replace vehicle history file {}: {}", path, err);
    }
}

fn la_seconds_past_midnight_from_epoch(epoch_secs: u64) -> u32 {
    let system_time = UNIX_EPOCH + Duration::from_secs(epoch_secs);
    let utc_datetime: chrono::DateTime<chrono::Utc> = system_time.into();
    let la_datetime = utc_datetime.with_timezone(&chrono_tz::America::Los_Angeles);
    la_datetime.hour() * 3600 + la_datetime.minute() * 60 + la_datetime.second()
}

fn la_start_date_string_from_epoch(epoch_secs: u64) -> String {
    let system_time = UNIX_EPOCH + Duration::from_secs(epoch_secs);
    let utc_datetime: chrono::DateTime<chrono::Utc> = system_time.into();
    utc_datetime
        .with_timezone(&chrono_tz::America::Los_Angeles)
        .format("%Y%m%d")
        .to_string()
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6_371_000.0_f64;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();

    let a = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    r * c
}

fn route_id_matches(transloc_route_id: i32, gtfs_route_id: &str) -> bool {
    let raw_route_id = transloc_route_id.to_string();
    let prefixed_route_id = format!("TL-{}", raw_route_id);
    gtfs_route_id == raw_route_id || gtfs_route_id == prefixed_route_id
}

fn stop_time_secs(stop_time: &StopTime) -> Option<u32> {
    stop_time.departure_time.or(stop_time.arrival_time)
}

fn trip_start_secs(trip: &Trip) -> Option<u32> {
    trip.stop_times.first().and_then(stop_time_secs)
}

fn trip_end_secs(trip: &Trip) -> Option<u32> {
    trip.stop_times.iter().rev().find_map(stop_time_secs)
}

fn candidate_in_current_window(start_time: i32, trip_duration: i32, service_seconds: i32) -> bool {
    service_seconds >= start_time - TRIP_MATCH_WINDOW_BEFORE_SECS
        && service_seconds <= start_time + trip_duration + TRIP_MATCH_WINDOW_AFTER_SECS
}

fn candidate_trip_instances(
    transloc_route_id: i32,
    gtfs: &Gtfs,
    service_seconds: u32,
) -> Vec<CandidateTrip> {
    let mut candidates = vec![];
    let mut seen = HashSet::new();
    let service_seconds = service_seconds as i32;

    for trip in gtfs
        .trips
        .values()
        .filter(|trip| route_id_matches(transloc_route_id, &trip.route_id))
    {
        let Some(base_start) = trip_start_secs(trip) else {
            continue;
        };
        let trip_duration = trip_end_secs(trip)
            .map(|end| (end as i32 - base_start as i32).max(0))
            .unwrap_or(0);

        let start_time = base_start;
        if candidate_in_current_window(start_time as i32, trip_duration, service_seconds) {
            let key = format!("{}|{}|{}", trip.route_id, trip.id, start_time);
            if seen.insert(key) {
                candidates.push(CandidateTrip {
                    trip_id: trip.id.clone(),
                    route_id: trip.route_id.clone(),
                    start_time,
                    schedule_start_time: start_time,
                });
            }
        }
    }

    candidates
}

fn current_vehicle_sample(vehicle: &AnteaterExpressData, now: u64) -> PositionSample {
    PositionSample {
        timestamp: now,
        latitude: vehicle.Latitude,
        longitude: vehicle.Longitude,
        speed_mps: Some(vehicle.GroundSpeed * (1.0 / 3.6)),
        heading: Some(vehicle.Heading),
        transloc_route_id: vehicle.RouteID,
    }
}

fn append_position_sample(history: &mut VehicleHistory, vehicle: &AnteaterExpressData, now: u64) {
    if history.last_transloc_route_id.is_some()
        && history.last_transloc_route_id != Some(vehicle.RouteID)
    {
        // Vehicle IDs can be reused or vehicles can be reassigned. Do not let a
        // route-5 history influence a future route-7 match for the same vehicle.
        history.positions.clear();
        history.current_delay_secs = 0;
        history.assigned_trip_id = None;
        history.assigned_route_id = None;
        history.assigned_start_time = None;
        history.matched_stop_sequence = None;
        history.matched_stop_id = None;
    }

    let sample = current_vehicle_sample(vehicle, now);
    let should_push = history.positions.last().map_or(true, |last| {
        now.saturating_sub(last.timestamp) >= HISTORY_MIN_SAMPLE_INTERVAL_SECS
            || haversine_m(
                last.latitude as f64,
                last.longitude as f64,
                sample.latitude as f64,
                sample.longitude as f64,
            ) >= HISTORY_MIN_SAMPLE_DISTANCE_M
    });

    if should_push {
        history.positions.push(sample);
    }

    history.positions.retain(|sample| {
        sample.transloc_route_id == vehicle.RouteID
            && now.saturating_sub(sample.timestamp) <= HISTORY_MAX_AGE_SECS
    });

    if history.positions.len() > HISTORY_MAX_SAMPLES {
        let remove_count = history.positions.len() - HISTORY_MAX_SAMPLES;
        history.positions.drain(0..remove_count);
    }

    history.last_transloc_route_id = Some(vehicle.RouteID);
    history.last_seen = now;
}

fn best_stop_match_for_sample(
    sample: &PositionSample,
    trip: &Trip,
    base_start_secs: i32,
    candidate_start_secs: i32,
    observed_secs: i32,
) -> Option<StopMatch> {
    let mut best: Option<StopMatch> = None;

    for stop_time in &trip.stop_times {
        let Some(template_stop_secs) = stop_time_secs(stop_time) else {
            continue;
        };

        let stop_offset = template_stop_secs as i32 - base_start_secs;
        let scheduled_secs = candidate_start_secs + stop_offset;
        let time_error_secs = (observed_secs - scheduled_secs).abs() as f64;
        let (Some(stop_latitude), Some(stop_longitude)) =
            (stop_time.stop.latitude, stop_time.stop.longitude)
        else {
            continue;
        };

        let distance_m = haversine_m(
            sample.latitude as f64,
            sample.longitude as f64,
            stop_latitude,
            stop_longitude,
        );

        // One point is roughly 45 seconds of schedule mismatch or 50 meters of
        // stop-location mismatch. This lets schedule and geography both matter.
        let score = (time_error_secs / 45.0) + (distance_m / 50.0);

        let stop_match = StopMatch {
            stop_sequence: stop_time.stop_sequence as u32,
            stop_id: stop_time.stop.id.clone(),
            distance_m,
            scheduled_secs,
            observed_secs,
            score,
        };

        if best
            .as_ref()
            .map(|existing| stop_match.score < existing.score)
            .unwrap_or(true)
        {
            best = Some(stop_match);
        }
    }

    best
}

fn score_candidate_trip(
    candidate: &CandidateTrip,
    trip: &Trip,
    history: Option<&VehicleHistory>,
    vehicle: &AnteaterExpressData,
    now: u64,
) -> Option<TripMatch> {
    let base_start_secs = trip_start_secs(trip)? as i32;
    let mut samples: Vec<PositionSample> = history
        .map(|history| {
            let mut samples = history
                .positions
                .iter()
                .rev()
                .filter(|sample| {
                    sample.transloc_route_id == vehicle.RouteID
                        && now.saturating_sub(sample.timestamp) <= MATCH_SAMPLE_MAX_AGE_SECS
                })
                .take(MATCH_SAMPLE_LIMIT.saturating_sub(1))
                .cloned()
                .collect::<Vec<_>>();
            samples.reverse();
            samples
        })
        .unwrap_or_default();

    // Always include the latest point even when it was not persisted because it
    // arrived less than HISTORY_MIN_SAMPLE_INTERVAL_SECS after the last sample.
    samples.push(current_vehicle_sample(vehicle, now));

    let mut total_score = 0.0_f64;
    let mut total_weight = 0.0_f64;
    let mut delay_samples = vec![];
    let mut newest_match: Option<(u64, StopMatch)> = None;

    for sample in samples {
        let observed_secs = la_seconds_past_midnight_from_epoch(sample.timestamp) as i32;
        let Some(stop_match) = best_stop_match_for_sample(
            &sample,
            trip,
            base_start_secs,
            candidate.start_time as i32,
            observed_secs,
        ) else {
            continue;
        };

        let age_secs = now.saturating_sub(sample.timestamp);
        let age_weight = 1.0 / (1.0 + age_secs as f64 / 600.0);
        total_score += stop_match.score * age_weight;
        total_weight += age_weight;

        // Delay is only meaningful when the vehicle is reasonably close to a
        // known stop. The newest match is still used as a fallback below.
        if stop_match.distance_m <= 150.0 {
            delay_samples.push(
                (stop_match.observed_secs - stop_match.scheduled_secs)
                    .clamp(-MAX_REASONABLE_DELAY_SECS, MAX_REASONABLE_DELAY_SECS),
            );
        }

        if newest_match
            .as_ref()
            .map(|(timestamp, _)| sample.timestamp > *timestamp)
            .unwrap_or(true)
        {
            newest_match = Some((sample.timestamp, stop_match));
        }
    }

    if total_weight == 0.0 {
        return None;
    }

    let mut average_score = total_score / total_weight;

    // Small hysteresis so a vehicle does not bounce between adjacent frequency
    // instances when its score is effectively tied with its previous assignment.
    if let Some(history) = history {
        if history.assigned_trip_id.as_deref() == Some(candidate.trip_id.as_str())
            && history.assigned_route_id.as_deref() == Some(candidate.route_id.as_str())
            && history.assigned_start_time == Some(candidate.start_time)
        {
            average_score *= 0.90;
        }
    }

    delay_samples.sort_unstable();
    let delay_secs = if !delay_samples.is_empty() {
        delay_samples[delay_samples.len() / 2]
    } else if let Some((_, newest)) = &newest_match {
        (newest.observed_secs - newest.scheduled_secs)
            .clamp(-MAX_REASONABLE_DELAY_SECS, MAX_REASONABLE_DELAY_SECS)
    } else {
        0
    };

    let current_stop_sequence = newest_match.as_ref().map(|(_, m)| m.stop_sequence);
    let stop_id = newest_match.as_ref().map(|(_, m)| m.stop_id.clone());

    Some(TripMatch {
        trip_id: candidate.trip_id.clone(),
        route_id: candidate.route_id.clone(),
        start_time: candidate.start_time,
        schedule_start_time: candidate.schedule_start_time,
        score: average_score,
        delay_secs,
        current_stop_sequence,
        stop_id,
    })
}

fn rank_trip_matches_for_vehicle(
    vehicle: &AnteaterExpressData,
    history: Option<&VehicleHistory>,
    gtfs: &Gtfs,
    now: u64,
    service_seconds: u32,
) -> Vec<TripMatch> {
    let mut ranked = candidate_trip_instances(vehicle.RouteID, gtfs, service_seconds)
        .into_iter()
        .filter_map(|candidate| {
            gtfs.trips
                .get(&candidate.trip_id)
                .and_then(|trip| score_candidate_trip(&candidate, trip, history, vehicle, now))
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal));
    ranked.truncate(MAX_CANDIDATES_PER_VEHICLE);
    ranked
}

fn select_unique_trip_matches(
    vehicles: &[AnteaterExpressData],
    history: &HashMap<i16, VehicleHistory>,
    gtfs: &Gtfs,
    now: u64,
) -> HashMap<i16, TripMatch> {
    let service_seconds = la_seconds_past_midnight_from_epoch(now);
    let mut ranked_by_vehicle: HashMap<i16, Vec<TripMatch>> = HashMap::new();
    let mut edges: Vec<(i16, usize, f64)> = vec![];

    for vehicle in vehicles {
        let ranked = rank_trip_matches_for_vehicle(
            vehicle,
            history.get(&vehicle.VehicleID),
            gtfs,
            now,
            service_seconds,
        );

        for (index, trip_match) in ranked.iter().enumerate() {
            edges.push((vehicle.VehicleID, index, trip_match.score));
        }

        ranked_by_vehicle.insert(vehicle.VehicleID, ranked);
    }

    edges.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal));

    let mut assigned = HashMap::new();
    let mut used_trip_instances = HashSet::new();

    for (vehicle_id, match_index, _) in edges {
        if assigned.contains_key(&vehicle_id) {
            continue;
        }

        let Some(matches) = ranked_by_vehicle.get(&vehicle_id) else {
            continue;
        };
        let Some(trip_match) = matches.get(match_index) else {
            continue;
        };

        if used_trip_instances.insert(trip_match.instance_key()) {
            assigned.insert(vehicle_id, trip_match.clone());
        }
    }

    assigned
}

fn current_vehicle_position(vehicle: &AnteaterExpressData) -> Position {
    Position {
        latitude: vehicle.Latitude,
        longitude: vehicle.Longitude,
        bearing: Some(vehicle.Heading),
        odometer: None,
        speed: Some(vehicle.GroundSpeed * (1.0 / 3.6)),
    }
}

async fn update_feeds(state: Arc<AppState>) {
    let mut last_history_save = 0_u64;

    loop {
        // Fetch new json.
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

                        let start_date_str = la_start_date_string_from_epoch(now);
                        let mut positions = vec![];
                        let mut trip_updates = vec![];
                        let mut history_snapshot_to_save: Option<HashMap<i16, VehicleHistory>> =
                            None;

                        {
                            let mut history_write = state.history.write().await;

                            for vehicle in &data {
                                let v_hist = history_write
                                    .entry(vehicle.VehicleID)
                                    .or_insert_with(VehicleHistory::default);
                                append_position_sample(v_hist, vehicle, now);
                            }

                            history_write.retain(|_, history| {
                                now.saturating_sub(history.last_seen) <= HISTORY_MAX_AGE_SECS * 6
                            });

                            let trip_matches = select_unique_trip_matches(
                                &data,
                                &*history_write,
                                &state.gtfs,
                                now,
                            );

                            for vehicle in &data {
                                if let Some(v_hist) = history_write.get_mut(&vehicle.VehicleID) {
                                    if let Some(trip_match) = trip_matches.get(&vehicle.VehicleID) {
                                        v_hist.current_delay_secs = trip_match.delay_secs;
                                        v_hist.assigned_trip_id = Some(trip_match.trip_id.clone());
                                        v_hist.assigned_route_id =
                                            Some(trip_match.route_id.clone());
                                        v_hist.assigned_start_time = Some(trip_match.start_time);
                                        v_hist.matched_stop_sequence =
                                            trip_match.current_stop_sequence;
                                        v_hist.matched_stop_id = trip_match.stop_id.clone();
                                    } else {
                                        v_hist.current_delay_secs = 0;
                                        v_hist.assigned_trip_id = None;
                                        v_hist.assigned_route_id = None;
                                        v_hist.assigned_start_time = None;
                                        v_hist.matched_stop_sequence = None;
                                        v_hist.matched_stop_id = None;
                                    }
                                }
                            }

                            for vehicle in &data {
                                let trip_match = trip_matches.get(&vehicle.VehicleID);
                                let gtfs_route_id = trip_match
                                    .map(|matched| matched.route_id.clone())
                                    .unwrap_or_else(|| vehicle.RouteID.to_string());

                                let trip_desc = TripDescriptor {
                                    trip_id: trip_match.map(|matched| matched.trip_id.clone()),
                                    route_id: Some(gtfs_route_id),
                                    direction_id: Some(0),
                                    start_time: trip_match
                                        .map(|matched| format_gtfs_time(matched.schedule_start_time)),
                                    start_date: Some(start_date_str.clone()),
                                    schedule_relationship: None,
                                    modified_trip: None,
                                };

                                let vehicle_desc = VehicleDescriptor {
                                    id: Some(vehicle.VehicleID.to_string()),
                                    label: Some(vehicle.Name.clone()),
                                    license_plate: None,
                                    wheelchair_accessible: None,
                                };

                                let current_pos = current_vehicle_position(vehicle);
                                let mut stop_time_updates = vec![];

                                if let Some(trip_match) = trip_match {
                                    if let Some(trip) = state.gtfs.trips.get(&trip_match.trip_id) {
                                        use gtfs_realtime::trip_update::StopTimeEvent;
                                        use gtfs_realtime::trip_update::StopTimeUpdate;

                                        for stop_time in &trip.stop_times {
                                            let seq = stop_time.stop_sequence as u32;
                                            let mut arrival = None;
                                            let mut departure = None;

                                            if stop_time.arrival_time.is_some() {
                                                arrival = Some(StopTimeEvent {
                                                    delay: Some(trip_match.delay_secs),
                                                    time: None,
                                                    uncertainty: None,
                                                });
                                            }

                                            if stop_time.departure_time.is_some() {
                                                departure = Some(StopTimeEvent {
                                                    delay: Some(trip_match.delay_secs),
                                                    time: None,
                                                    uncertainty: None,
                                                });
                                            }

                                            stop_time_updates.push(StopTimeUpdate {
                                                stop_sequence: Some(seq),
                                                stop_id: Some(stop_time.stop.id.clone()),
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
                                        current_stop_sequence: trip_match
                                            .and_then(|matched| matched.current_stop_sequence),
                                        stop_id: trip_match
                                            .and_then(|matched| matched.stop_id.clone()),
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

                                if let Some(trip_match) = trip_match {
                                    trip_updates.push(FeedEntity {
                                        id: format!("tu_{}", vehicle.VehicleID),
                                        is_deleted: Some(false),
                                        trip_update: Some(TripUpdate {
                                            trip: trip_desc,
                                            vehicle: Some(vehicle_desc),
                                            stop_time_update: stop_time_updates,
                                            timestamp: Some(now),
                                            delay: Some(trip_match.delay_secs),
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

                            if now.saturating_sub(last_history_save) >= HISTORY_SAVE_INTERVAL_SECS {
                                history_snapshot_to_save = Some(history_write.clone());
                                last_history_save = now;
                            }
                        }

                        if let Some(history_snapshot) = history_snapshot_to_save {
                            save_history_to_disk(&history_snapshot).await;
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
    let history = load_history_from_disk().await;

    let state = Arc::new(AppState {
        gtfs,
        positions_feed: RwLock::new(vec![]),
        trip_updates_feed: RwLock::new(vec![]),
        history: RwLock::new(history),
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
