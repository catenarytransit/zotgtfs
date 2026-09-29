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

/// Represents a distinct temporal segment of the transit schedule.
struct ServiceWindow {
    start_sec: u32,
    end_sec: u32,
    headway_sec: u32,
    service_id: &'static str,
}

pub fn redo_anteater_express_gtfs(mut gtfs: Gtfs) -> Gtfs {
    // Helper to find a stop by ID or Stop Code (UCI GTFS can be inconsistent)
    let find_stop = |gtfs: &Gtfs, identifier: &str| {
        gtfs.stops.get(identifier).cloned().or_else(|| {
            gtfs.stops
                .values()
                .find(|s| s.code.as_deref() == Some(identifier))
                .cloned()
        })
    };

    // Campus-California (stop_code 106) is missing from some source GTFS
    // exports. Add it before rebuilding trips so the E Line's existing 106
    // stop entry is not silently skipped by find_stop().
    if find_stop(&gtfs, "106").is_none() {
        let campus_california = gtfs_structures::Stop {
            id: "106".to_string(),
            code: Some("106".to_string()),
            name: Some("Campus-California".to_string()),
            latitude: Some(33.648832),
            longitude: Some(-117.829686),
            ..gtfs_structures::Stop::default()
        };

        gtfs.stops
            .insert("106".to_string(), Arc::new(campus_california));
    }

    // 1. Identify and extract templates
    let mut templates = HashMap::new();
    let route_configs = [
        ("A Line", "107"), // Using codes/IDs common in UCI GTFS
        ("E Line", "100"),
        ("H Line", "100"),
        ("M Line", "100"),
        ("N Line", "107"),
    ];

    for (line_name, primary_stop_id) in route_configs {
        if let Some(route) = gtfs.routes.values().find(|r| {
            r.long_name
                .as_deref()
                .unwrap_or("")
                .eq_ignore_ascii_case(line_name)
        }) {
            // Pick a template trip that actually HAS stop times
            if let Some(mut template) = gtfs
                .trips
                .values()
                .filter(|t| t.route_id == route.id && !t.stop_times.is_empty())
                .max_by_key(|t| t.stop_times.len())
                .cloned()
            {
                // Overwrite H-Line sequence
                if line_name == "E Line" {
                    if let Some(default_st) = template.stop_times.first().cloned() {
                        template.stop_times.clear();
                        let e_stops = vec![("100", 0), ("101", 60), ("106", 120), ("100", 600)];
                        for (i, (sid, offset)) in e_stops.into_iter().enumerate() {
                            if let Some(stop) = find_stop(&gtfs, sid) {
                                let mut st = default_st.clone();
                                st.stop = stop;
                                st.arrival_time = Some(offset);
                                st.departure_time = Some(offset);
                                st.stop_sequence = i as u32;
                                template.stop_times.push(st);
                            }
                        }
                    }
                }
                if line_name == "H Line" {
                    if let Some(default_st) = template.stop_times.first().cloned() {
                        template.stop_times.clear();
                        let h_stops = vec![
                            ("100", 0),
                            ("101", 240),
                            ("108", 240),
                            ("109", 300),
                            ("110", 600),
                            ("111", 600),
                            ("112", 600),
                            ("103", 840),
                            ("104", 840),
                            ("118", 960),
                            ("124", 960),
                            ("125", 960),
                            ("126", 1320),
                            ("113", 1440),
                            ("100", 1800),
                        ];
                        for (i, (sid, offset)) in h_stops.into_iter().enumerate() {
                            if let Some(stop) = find_stop(&gtfs, sid) {
                                let mut st = default_st.clone();
                                st.stop = stop;
                                st.arrival_time = Some(offset);
                                st.departure_time = Some(offset);
                                st.stop_sequence = i as u32;
                                template.stop_times.push(st);
                            }
                        }
                    }
                }

                // Final safety: ensure every stop time has an arrival/departure
                // if it was missing in the template, we'll use 0 as a base.
                for st in template.stop_times.iter_mut() {
                    if st.arrival_time.is_none() {
                        st.arrival_time = Some(0);
                    }
                    if st.departure_time.is_none() {
                        st.departure_time = st.arrival_time;
                    }
                }

                templates.insert(line_name, (route.id.clone(), template, primary_stop_id));
            }
        }
    }

    // 2. Clear and Rebuild
    gtfs.trips.clear();
    let mut new_trips = HashMap::new();

    for (line_name, (route_id, template, primary_stop_id)) in templates {
        // Find the anchor offset in the template trip
        let anchor_offset = template
            .stop_times
            .iter()
            .find(|st| {
                st.stop.id == primary_stop_id || st.stop.code.as_deref() == Some(primary_stop_id)
            })
            .and_then(|st| st.arrival_time)
            .unwrap_or(0);

        let windows = match line_name {
            "A Line" => vec![
                ServiceWindow {
                    start_sec: 27480,
                    end_sec: 37980,
                    headway_sec: 480,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 37980,
                    end_sec: 67980,
                    headway_sec: 780,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 27480,
                    end_sec: 37980,
                    headway_sec: 480,
                    service_id: "TL-13",
                },
                ServiceWindow {
                    start_sec: 37980,
                    end_sec: 56700,
                    headway_sec: 780,
                    service_id: "TL-13",
                },
            ],
            "M Line" => vec![
                ServiceWindow {
                    start_sec: 27900,
                    end_sec: 67380,
                    headway_sec: 480,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 67380,
                    end_sec: 71700,
                    headway_sec: 780,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 71700,
                    end_sec: 81000,
                    headway_sec: 1500,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 27900,
                    end_sec: 57900,
                    headway_sec: 480,
                    service_id: "TL-13",
                },
                ServiceWindow {
                    start_sec: 57900,
                    end_sec: 70200,
                    headway_sec: 1500,
                    service_id: "TL-13",
                },
            ],
            "N Line" => vec![
                ServiceWindow {
                    start_sec: 27300,
                    end_sec: 57300,
                    headway_sec: 420,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 57300,
                    end_sec: 68100,
                    headway_sec: 600,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 27300,
                    end_sec: 56880,
                    headway_sec: 420,
                    service_id: "TL-13",
                },
            ],
            "H Line" => vec![
                ServiceWindow {
                    start_sec: 68400,
                    end_sec: 81000,
                    headway_sec: 600,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 57600,
                    end_sec: 70200,
                    headway_sec: 600,
                    service_id: "TL-13",
                },
            ],
            "E Line" => vec![
                ServiceWindow {
                    start_sec: 27600,
                    end_sec: 67800,
                    headway_sec: 600,
                    service_id: "TL-12",
                },
                ServiceWindow {
                    start_sec: 27600,
                    end_sec: 57000,
                    headway_sec: 600,
                    service_id: "TL-13",
                },
            ],
            _ => vec![],
        };

        for window in windows {
            let mut current_start = window.start_sec;
            while current_start <= window.end_sec {
                let mut trip = template.clone();
                let trip_id = format!("{}-{}-{}", route_id, window.service_id, current_start);

                trip.id = trip_id.clone();
                trip.service_id = window.service_id.to_string();
                trip.frequencies.clear();

                let time_shift = current_start as i32 - anchor_offset as i32;
                let mut passed_headsign_split = false;

                for st in trip.stop_times.iter_mut() {
                    // Shift arrival and departure, ensuring they are never negative
                    st.arrival_time = st
                        .arrival_time
                        .map(|t| (t as i32 + time_shift).max(0) as u32);
                    st.departure_time = st.arrival_time;

                    // stop_headsign overrides trip_headsign for this specific stop_time.
                    // The split stop itself gets the return headsign because the bus is
                    // departing that stop toward the second half of the loop.
                    let stop_code = st.stop.code.as_deref().unwrap_or(st.stop.id.as_str());

                    st.stop_headsign = match line_name {
                        "H Line" | "A Line" => {
                            if stop_code == "103" {
                                passed_headsign_split = true;
                            }

                            Some(
                                if passed_headsign_split {
                                    if line_name == "H Line" {
                                        "University Center South"
                                    } else {
                                        "University Center North"
                                    }
                                } else {
                                    "Vista del Campo (VDC)"
                                }
                                .to_string(),
                            )
                        }
                        "N Line" => {
                            if stop_code == "118" {
                                passed_headsign_split = true;
                            }

                            Some(
                                if passed_headsign_split {
                                    "University Center North"
                                } else {
                                    "VDC Norte"
                                }
                                .to_string(),
                            )
                        }
                        "M Line" => {
                            if stop_code == "161" {
                                passed_headsign_split = true;
                            }

                            Some(
                                if passed_headsign_split {
                                    "University Center South"
                                } else {
                                    "Engineering"
                                }
                                .to_string(),
                            )
                        }
                        "E Line" => {
                            if stop_code == "106" {
                                passed_headsign_split = true;
                            }

                            Some(
                                if passed_headsign_split {
                                    "University Center South"
                                } else {
                                    "Plaza Verde"
                                }
                                .to_string(),
                            )
                        }
                        _ => st.stop_headsign.clone(),
                    };
                }

                new_trips.insert(trip_id, trip);
                current_start += window.headway_sec;
            }
        }
    }

    gtfs.trips = new_trips;
    gtfs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_the_gtfs() {
        let gtfs = Gtfs::from_path("original_gtfs.zip").expect("Failed to load GTFS");
        let modified_gtfs = redo_anteater_express_gtfs(gtfs);
        assert!(!modified_gtfs.trips.is_empty(), "No trips generated");
    }
}
