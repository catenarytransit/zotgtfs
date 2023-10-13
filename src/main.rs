
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;
use protobuf::{CodedInputStream, Message as ProtobufMessage};
use prost::Message;
use std::time::UNIX_EPOCH;
use gtfs_rt::EntitySelector;
use gtfs_rt::TimeRange;
use serde_json;

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

fn arrival_estimates_length_to_end(bus: &EachBus) -> i32 {
    let mut length = 0;

    for estimate in bus.arrival_estimates.iter() {
        if estimate.stop_id.is_some() {
            if estimate.stop_id.unwrap().as_str() == "8197566" || estimate.stop_id.unwrap().as_str() == "8274064" {
                break;
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

    let client = reqwest::Client::new();

    loop {

        let mut list_of_vehicle_positions: Vec<gtfs_rt::FeedEntity> = Vec::new();

        let beginning = Instant::now();

        let res = client.get("https://transloc-api-1-2.p.rapidapi.com/vehicles.json?agencies=1039")
            .header("X-Mashape-Key", "b0ebd9e8a5msh5aca234d74ce282p1737bbjsnddd18d7b9365")
            .send()
            .await
            .unwrap();

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

        for (route_id, buses) in grouped_by_route.iter() {
            //let sort the buses by completion

            let mut sorted_buses = buses.clone();
            
            sorted_buses.sort_by(|bus_a, bus_b| arrival_estimates_length_to_end(bus_b).cmp(&arrival_estimates_length_to_end(bus_a)));

            println!("order of completion [{}]: {:?}", route_id, sorted_buses.into_iter().map(|x| x.arrival_estimates.len()).collect::<Vec<usize>>());
        }

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

                    let vehicleposition = gtfs_rt::FeedEntity {
                        id: bus.vehicle_id.as_ref().unwrap().clone(),
                        vehicle: Some(
                            gtfs_rt::VehiclePosition {
                                trip: Some(gtfs_rt::TripDescriptor {
                                    trip_id: Some("GoAnteaters!".to_string()),
                                    route_id: Some(bus.route_id.as_ref().unwrap().clone()),
                                    direction_id: Some(0),
                                    start_time: None,
                                    start_date: Some(chrono::Utc::now().format("%Y%m%d").to_string()),
                                    schedule_relationship: None,
                                }),
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

       // println!("Encoded to protobuf! {:#?}", entire_feed_vehicles);

        //let entire_feed_vehicles = entire_feed_vehicles.encode_to_vec();

        let buf:Vec<u8> = entire_feed_vehicles.encode_to_vec();

                        let _: () = con
                                        .set(
                                            format!(
                                                "gtfsrt|{}|{}",
                                                "f-anteaterexpress~rt", "vehicles"
                                            ),
                                            &buf,
                                        )
                                        .unwrap();

                                        let _: () = con
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

                                        let _: () = con
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
