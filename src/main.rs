
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;
use protobuf::{CodedInputStream, Message as ProtobufMessage};
use prost::Message;
use std::time::UNIX_EPOCH;
use gtfs_rt::EntitySelector;
use chrono::Datelike;
use chrono_tz::US::Pacific;
use gtfs_rt::TimeRange;
use serde_json;
use chrono::TimeZone;
use chrono::Timelike;
use chrono_tz::Tz;
use chrono_tz::UTC;


use redis::Commands;
use redis::RedisError;
use redis::{Client as RedisClient, RedisResult};

use std::time::{Duration, SystemTime};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TranslocRealtime {
    rate_limit: u32,
    expires_in: u32,
    api_latest_version: String,
    generated_on: String,
    data: BTreeMap<String, Vec<EachBus>>,
    api_version: String
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct EachBus {
    description: Option<String>,
    passenger_load: Option<i32>,
    standing_capacity: Option<i32>,
    seating_capacity: Option<i32>,
    last_updated_on: String,
    call_name: Option<String>,
    speed: Option<f32>,
    vehicle_id: Option<String>,
    segment_id: Option<String>,
    route_id: Option<String>,
    tracking_status: Option<String>,
    arrival_estimates: Vec<ArrivalEstimates>,
    location: TranslocLocation,
    heading: Option<f32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ArrivalEstimates {
    route_id: Option<String>,
    arrival_at: Option<String>,
    stop_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TranslocLocation {
    lat: f32,
    lng: f32
}

fn allowtrip(trip_id: &String, trip: &gtfs_structures::Trip, route_id: &String, gtfs: &gtfs_structures::Gtfs) -> bool {


    let calendarselected = gtfs.calendar.get(trip.service_id.as_str()).unwrap();
            
    //is it friday in Los Angeles?
    // Get the timezone for Los Angeles.
let current_time = chrono::Utc::now();

let tz: Tz = "America/Los_Angeles".parse().unwrap();

// Convert this to the Los Angeles timezone.

let current_time_la = current_time.with_timezone(&tz);

let is_friday = current_time_la.weekday() == chrono::Weekday::Fri;

let current_time_in_seconds = (current_time_la.hour() * 3600) + (current_time_la.minute() * 60) + current_time_la.second();

if trip.stop_times[0].departure_time.is_some() {
    let departure_time = trip.stop_times[0].departure_time.as_ref().unwrap();

    let diff = *departure_time as i32 - current_time_in_seconds as i32;
    //large time means the trip hasn't started yet
    //negative time means the trip has already started 

    if diff > 1500 || diff < -3600 {
        return false;
    }
}

return match is_friday {
    true => {
        calendarselected.friday == true && trip.route_id == *route_id
    },
    false => {
        calendarselected.monday == true && trip.route_id == *route_id
    }
    }
}

fn arrival_estimates_length_to_end(bus: &EachBus) -> i32 {
    let mut length = 0;

    'estimation: for estimate in bus.arrival_estimates.iter() {
        if estimate.stop_id.is_some() {
          /*
            if estimate.stop_id.as_ref().unwrap().as_str() == "8197566" || estimate.stop_id.as_ref().unwrap().as_str() == "8274064" {
                break 'estimation;
            }
             */
        }

        if estimate.route_id.is_some() {
            if estimate.route_id.as_ref().unwrap().as_str() != bus.route_id.as_ref().unwrap().as_str() {
                break 'estimation;
            }
        }

        if estimate.arrival_at.is_some() {
            length += 1;
        }
    }

    return length;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    color_eyre::install()?;
   // curl https://transloc-api-1-2.p.rapidapi.com/vehicles.json?agencies=1039 
   //-H "X-Mashape-Key: b0ebd9e8a5msh5aca234d74ce282p1737bbjsnddd18d7b9365"

   let redisclient = RedisClient::open("redis://127.0.0.1:6379/").unwrap();
   let mut con = redisclient.get_connection().unwrap();

    let gtfs = gtfs_structures::GtfsReader::default()
    .read("anteater_gtfs").unwrap();

    let rawgtfs = gtfs_structures::RawGtfs::new("anteater_gtfs").unwrap();

    let client = reqwest::Client::new();

    loop {

        let mut list_of_vehicle_positions: Vec<gtfs_rt::FeedEntity> = Vec::new();
        let mut listoftripupdates: Vec<gtfs_rt::FeedEntity> = Vec::new();

        let beginning = Instant::now();

        let res = client.get("https://transloc-api-1-2.p.rapidapi.com/vehicles.json?agencies=1039")
            .header("X-Mashape-Key", "5a51a5d8eamsh957095ccebfa10dp111874jsnafe6b4225d3d")
            .send()
            .await
            .unwrap();

        if res.status() != 200 {
            println!("Error: {}", res.status());
            continue;
        }

        println!("Downloaded {} chars", res.content_length().unwrap());

        let body = res.text().await.unwrap();

        let import_data: TranslocRealtime = serde_json::from_str(body.as_str()).unwrap();

        let mut vehicle_id_to_trip_id:HashMap<String, String> = HashMap::new();

        let mut grouped_by_route: HashMap<String, Vec<EachBus>> = HashMap::new();

        import_data.data.iter().for_each(|(agency_id, buses)| {
            if agency_id.as_str() == "1039" {
                for (i, bus) in buses.iter().enumerate() {
                    if bus.route_id.is_some() {
                        if grouped_by_route.contains_key(bus.route_id.as_ref().unwrap()) {
                            grouped_by_route.get_mut(bus.route_id.as_ref().unwrap()).unwrap().push(bus.clone());
                        } else {
                            grouped_by_route.insert(bus.route_id.as_ref().unwrap().clone(), vec![bus.clone()]);
                        }
                    }
                }
            }
        });

        
let current_time = chrono::Utc::now();

let tz: Tz = "America/Los_Angeles".parse().unwrap();

// Convert this to the Los Angeles timezone.

let current_time_la = current_time.with_timezone(&tz);

let midnight = current_time_la.date().and_hms(0, 0, 0);

let midnight_timestamp = midnight.timestamp();

let mut delay_hashmap: HashMap<String, i32> = HashMap::new();

        for (route_id, buses) in grouped_by_route.iter() {
            //let sort the buses by completion

            let mut sorted_buses = buses.clone();
            
            sorted_buses.sort_by(|bus_a, bus_b| arrival_estimates_length_to_end(bus_a).cmp(&arrival_estimates_length_to_end(bus_b)));

            println!("order of completion [{}]: {:?}", route_id, &sorted_buses.iter().map(|x| arrival_estimates_length_to_end(&x)).collect::<Vec<i32>>());
            
            let mut possible_trips = gtfs.trips.iter().filter(|(trip_id,trip)| allowtrip(&trip_id, &trip, &route_id, &gtfs)).map(|(trip_id, trip)| trip).collect::<Vec<&gtfs_structures::Trip>>();

            possible_trips.sort_by(|trip_a, trip_b| trip_a.id.cmp(&trip_b.id));

            println!("possible trips on Route {}: {:?}",gtfs.get_route(route_id).unwrap().short_name , possible_trips.iter().map(|x| x.id.clone()).collect::<Vec<String>>());

            for (i,bus) in (&sorted_buses).iter().rev().enumerate() {
                if possible_trips.len() == 0 {
                    vehicle_id_to_trip_id.insert(bus.vehicle_id.as_ref().unwrap().clone(), format!("extra-{}-{i}", bus.route_id.as_ref().unwrap().clone()));
                } else {
                    if possible_trips.len() == 1 {
                        vehicle_id_to_trip_id.insert(bus.vehicle_id.as_ref().unwrap().clone(), possible_trips[0].id.clone());
                    } else {
                        let assigned_id = possible_trips[0].id.clone();

                        let mut closest_past_trip: Option<String> = None;
                        let mut remove_before_index:Option<usize> = None;

                        let searchable_stop_times_from_bus = bus.arrival_estimates.iter().filter(|arrival_estimate| arrival_estimate.arrival_at.is_some() && arrival_estimate.stop_id.is_some()).collect::<Vec<&ArrivalEstimates>>();
                    
                        //println!("lineup {} vs {}", searchable_stop_times_from_gtfs.len(), searchable_stop_times_from_bus.len());

                        let mut timedifference:Option<i32> = None;

                        //search through the entire trip list
                        'search_trip_list: for (tripcounter,lookingtrip) in possible_trips.iter().rev().enumerate() {
                            let searchable_stop_times_from_gtfs = possible_trips[tripcounter].stop_times.iter().filter(|stop_time| stop_time.departure_time.is_some()).collect::<Vec<&gtfs_structures::StopTime>>();
                            let searchable_stop_times_stop_ids = searchable_stop_times_from_gtfs.iter().map(|stop_time| stop_time.stop.id.clone()).collect::<Vec<String>>();
                            
                            let searchable_stop_times_bus_filterable = searchable_stop_times_from_bus.iter().filter(|arrival_estimate| searchable_stop_times_stop_ids.contains(arrival_estimate.stop_id.as_ref().unwrap())).collect::<Vec<&&ArrivalEstimates>>();
                            
                           if searchable_stop_times_bus_filterable.len() > 0 {
                            let bus_arrival_timestamp = chrono::DateTime::parse_from_rfc3339(searchable_stop_times_bus_filterable[0].arrival_at.as_ref().unwrap()).unwrap().timestamp() - midnight_timestamp;

                           // println!("{}, {}", searchable_stop_times_from_gtfs[0].departure_time.as_ref().unwrap(), bus_arrival_timestamp);
                            let time_diff = *searchable_stop_times_from_gtfs[0].departure_time.as_ref().unwrap() as i32 - bus_arrival_timestamp as i32;
                            //positive means the bus would get there before the scheduled time
                            //negative means that it's late, as the projected arrival time is greater than the scheduled time

                            //bias algorithm towards late buses i think?
                           let score:f64 = {
                            if time_diff < 0 {
                                time_diff.abs() as f64 
                            } else {
                                time_diff.abs() as f64
                            }
                        };

                            if time_diff.abs() < 3600 {
                               println!("time diff: {}", time_diff);
                                match timedifference {
                                    Some(x) => {
                                        //if the previous trip comparison is worse
                                        if (x.abs() as f64) > score {
                                            timedifference = Some(time_diff);
                                            closest_past_trip = Some(possible_trips[tripcounter].id.clone());
                                            remove_before_index = Some(tripcounter);
                                            delay_hashmap.insert(bus.vehicle_id.as_ref().unwrap().clone(), time_diff);
                                        } else {
                                            break 'search_trip_list;
                                        }
                                    },
                                    None => {
                                        timedifference = Some(time_diff);
                                        closest_past_trip = Some(possible_trips[tripcounter].id.clone());
                                        delay_hashmap.insert(bus.vehicle_id.as_ref().unwrap().clone(), time_diff);
                                    }
                                }
                            } else {
                            }
                           } else {
                            println!("No trips left to search for {}", bus.vehicle_id.as_ref().unwrap().clone());
                           }
                        }

                        if remove_before_index.is_some() {
                            possible_trips.drain(0..remove_before_index.unwrap() + 1);
                        }
                        
                        let closest_past_trip = match closest_past_trip {
                            Some(x) => x,
                            None => {
                                String::from("GoAnteaters!")
                            }
                        };

                        println!("Route: {}, Bus: {} assigned to {}", gtfs.get_route(route_id).unwrap().short_name, bus.call_name.as_ref().unwrap(), &closest_past_trip);
                        vehicle_id_to_trip_id.insert(bus.vehicle_id.as_ref().unwrap().clone(), closest_past_trip);
                    }
                }
            }

            }

            println!("vehicle_id_to_trip_id: {:?}", vehicle_id_to_trip_id);
       

            println!("Delay Hashmap {:#?}", delay_hashmap);

        import_data.data.iter().for_each(|(agency_id, buses)| {
            if agency_id.as_str() == "1039" {
                for (i, bus) in buses.iter().enumerate() {

                    let bruhposition = Some(gtfs_rt::Position {
                        latitude: bus.location.lat,
                        longitude: bus.location.lng,
                        bearing: bus.heading,
                        odometer: None,
                        speed: Some((bus.speed.unwrap_or(0.0) as f32 * (1.0/3.6)) as f32),
                    });

                    let trip_ident = gtfs_rt::TripDescriptor {
                        trip_id: Some(vehicle_id_to_trip_id.get(bus.vehicle_id.as_ref().unwrap()).unwrap().clone()),
                        route_id: Some(bus.route_id.as_ref().unwrap().clone()),
                        direction_id: Some(0),
                        start_time: None,
                        start_date: Some(chrono::Utc::now().format("%Y%m%d").to_string()),
                        schedule_relationship: None,
                    };

                    let vehicleposition = gtfs_rt::FeedEntity {
                        id: bus.vehicle_id.as_ref().unwrap().clone(),
                        vehicle: Some(
                            gtfs_rt::VehiclePosition {
                                trip: Some(trip_ident.clone()),
                                vehicle: Some(gtfs_rt::VehicleDescriptor {
                                    id: Some(bus.vehicle_id.as_ref().unwrap().clone()),
                                    label: Some(bus.call_name.as_ref().unwrap().clone()),
                                    license_plate: None,
                                }),
                                position: bruhposition,
                                current_stop_sequence: None,
                                stop_id: None,
                                current_status: None,
                                timestamp: Some(bus.last_updated_on.parse::<chrono::DateTime<chrono::Utc>>().unwrap().timestamp() as u64),
                                congestion_level: None,
                                occupancy_status: None
                            }
                        ),
                        is_deleted: None,
                        trip_update: None,
                        alert: None
                    };

                    let this_trip_length = std::cmp::min(arrival_estimates_length_to_end(bus) + 2, bus.arrival_estimates.len() as i32);

                    let this_trip_updates:Vec<ArrivalEstimates> = bus.arrival_estimates[0..this_trip_length as usize].to_vec();

                    let time_updates:Vec<gtfs_rt::trip_update::StopTimeUpdate> = this_trip_updates.iter().map(|x| gtfs_rt::trip_update::StopTimeUpdate {
                        stop_sequence: None,
                        stop_id: x.stop_id.clone(),
                        //unix time
                        arrival: Some(gtfs_rt::trip_update::StopTimeEvent {
                            time: Some(chrono::DateTime::parse_from_rfc3339(x.arrival_at.as_ref().unwrap()).unwrap().timestamp()),
                            delay: None,
                            uncertainty: None
                            
                    }),
                        departure: None,
                        schedule_relationship: Some(0)
                    }).collect::<Vec<gtfs_rt::trip_update::StopTimeUpdate>>();

                    let tripupdate = gtfs_rt::FeedEntity {
                        id: bus.vehicle_id.as_ref().unwrap().clone(),
                        vehicle: None,
                        is_deleted: None,
                        trip_update: Some(
                           gtfs_rt::TripUpdate { trip: trip_ident, vehicle: 
                            Some(gtfs_rt::VehicleDescriptor {
                                id: Some(bus.vehicle_id.as_ref().unwrap().clone()),
                                label: Some(bus.call_name.as_ref().unwrap().clone()),
                                license_plate: None,
                            })
                            , stop_time_update: time_updates, timestamp:  Some(bus.last_updated_on.parse::<chrono::DateTime<chrono::Utc>>().unwrap().timestamp() as u64), delay: delay_hashmap.get(bus.vehicle_id.as_ref().unwrap()).map(|x| *x as i32), }
                        ),
                        alert: None
                    };

                    listoftripupdates.push(tripupdate);
                    list_of_vehicle_positions.push(vehicleposition);
                }
            }
        });

        let entire_feed_vehicles = gtfs_rt::FeedMessage {
            header: gtfs_rt::FeedHeader {
                gtfs_realtime_version: String::from("2.0"),
                incrementality: None,
                timestamp: Some(SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()),
            },
            entity: list_of_vehicle_positions,
        };

        let trip_feed = gtfs_rt::FeedMessage {
            header: gtfs_rt::FeedHeader {
                gtfs_realtime_version: String::from("2.0"),
                incrementality: None,
                timestamp: Some(SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()),
            },
            entity: listoftripupdates,
        };

       // println!("Encoded to protobuf! {:#?}", entire_feed_vehicles);

        //let entire_feed_vehicles = entire_feed_vehicles.encode_to_vec();

        let buf:Vec<u8> = entire_feed_vehicles.encode_to_vec();
        let trip_buf:Vec<u8> = trip_feed.encode_to_vec();

                        let _: () = con
                                        .set(
                                            format!(
                                                "gtfsrt|{}|{}",
                                                "f-anteaterexpress~rt", "vehicles"
                                            ),
                                            &buf,
                                        )
                                        .unwrap();

                                        let _:() = con
                                        .set(
                                            format!(
                                                "gtfsrt|{}|{}",
                                                "f-anteaterexpress~rt", "trips"
                                            ),
                                            &trip_buf,
                                        )
                                        .unwrap();

                                        let _:() = con
                                        .set(
                                            format!(
                                                "gtfsrttime|{}|{}",
                                                "f-anteaterexpress~rt", "vehicles"
                                            ),
                                            SystemTime::now()
                                                .duration_since(UNIX_EPOCH)
                                                .unwrap()
                                                .as_millis()
                                                .to_string(),
                                        )
                                        .unwrap();

                                        let _:() = con
                                        .set(
                                            format!(
                                                "gtfsrttime|{}|{}",
                                                "f-anteaterexpress~rt", "trips"
                                            ),
                                            SystemTime::now()
                                                .duration_since(UNIX_EPOCH)
                                                .unwrap()
                                                .as_millis()
                                                .to_string(),
                                        ).unwrap();

                                        let _ :()= con
                                        .set(
                                            format!(
                                                "gtfsrtexists|{}",
                                                "f-anteaterexpress~rt"
                                            ),
                                            SystemTime::now()
                                                .duration_since(UNIX_EPOCH)
                                                .unwrap()
                                                .as_millis()
                                                .to_string(),
                                        )
                                        .unwrap();

        println!("Inserted into Redis!");

        let time_left = 100 as f64 - (beginning.elapsed().as_millis() as f64);

        if time_left > 0.0 {
            println!("Sleeping for {} milliseconds", time_left);
            std::thread::sleep(std::time::Duration::from_millis(time_left as u64));
        }
    }
}
