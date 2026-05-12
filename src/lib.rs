use gtfs_realtime::vehicle_position::*;
use gtfs_realtime::*;
use serde::Deserialize;
use serde_json::from_str;
use std::error::Error;
use gtfs_structures::Gtfs;
use chrono::Datelike;
use chrono_tz::Tz;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn get_gtfs_rt(gtfs: &gtfs_structures::Gtfs) -> Result<gtfs_realtime::FeedMessage, Box<dyn std::error::Error + Send + Sync>> {
    let vehicle_data = reqwest::get("https://ucirvine.transloc.com/Services/JSONPRelay.svc/GetMapVehiclePoints?_=1712182850877")
        .await?
        .text()
        .await?;
    gtfs_rt_from_string(vehicle_data, gtfs)
}

fn get_trip_id(route_id: &str, gtfs: &gtfs_structures::Gtfs) -> Option<String> {
    let gtfs_route_id = format!("TL-{}", route_id);
    gtfs.trips.values()
        .find(|t| t.route_id == gtfs_route_id)
        .map(|t| t.id.clone())
}

fn gtfs_rt_from_string(
    vehicle_data: String,
    gtfs: &gtfs_structures::Gtfs
) -> Result<gtfs_realtime::FeedMessage, Box<dyn std::error::Error + Send + Sync>> {
    let data = parse_data(vehicle_data)?;
    if data.is_empty() {
        return Ok(gtfs_realtime::FeedMessage {
            header: FeedHeader {
                gtfs_realtime_version: String::from("2.0"),
                incrementality: None,
                timestamp: Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()),
            },
            entity: vec![],
        });
    }

    let mut anteater_entities: Vec<FeedEntity> = Vec::new();
    for (i, vehicle) in data.iter().enumerate() {
        anteater_entities.push(FeedEntity {
            id: i.to_string(),
            is_deleted: Some(false),
            trip_update: None,
            vehicle: Some(vehicle.get_vehicle_position(gtfs)),
            alert: None,
            shape: None,
            stop: None,
            trip_modifications: None,
        });
    }
    
    Ok(gtfs_realtime::FeedMessage {
        header: FeedHeader {
            gtfs_realtime_version: String::from("2.0"),
            incrementality: None,
            timestamp: Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()),
        },
        entity: anteater_entities,
    })
}

#[derive(Deserialize)]
struct AnteaterExpressData {
    #[serde(rename = "GroundSpeed")]
    ground_speed: f32,
    #[serde(rename = "Heading")]
    heading: f32,
    #[serde(rename = "Latitude")]
    latitude: f32,
    #[serde(rename = "Longitude")]
    longitude: f32,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "RouteID")]
    route_id: i32,
    #[serde(rename = "VehicleID")]
    vehicle_id: i16,
}

impl AnteaterExpressData {
    fn get_carriage_details(&self) -> CarriageDetails {
        CarriageDetails {
            id: Some(self.vehicle_id.to_string()),
            label: Some(self.name.clone()),
            occupancy_status: None,
            occupancy_percentage: None,
            carriage_sequence: Some(1),
        }
    }

    fn get_position(&self) -> Position {
        Position {
            latitude: self.latitude,
            longitude: self.longitude,
            bearing: Some(self.heading),
            odometer: None,
            speed: Some(self.ground_speed * (1.0 / 3.6)),
        }
    }

    fn get_trip_descriptor(&self, gtfs: &gtfs_structures::Gtfs) -> TripDescriptor {
        let route_str = self.route_id.to_string();
        TripDescriptor {
            trip_id: get_trip_id(&route_str, gtfs),
            route_id: Some(format!("TL-{}", self.route_id)),
            direction_id: Some(0),
            start_time: None,
            start_date: None,
            schedule_relationship: None,
            modified_trip: None,
        }
    }

    fn get_vehicle_descriptor(&self) -> VehicleDescriptor {
        VehicleDescriptor {
            id: Some(self.vehicle_id.to_string()),
            label: Some(self.name.clone()),
            license_plate: None,
            wheelchair_accessible: None,
        }
    }

    fn get_vehicle_position(&self, gtfs: &gtfs_structures::Gtfs) -> VehiclePosition {
        VehiclePosition {
            trip: Some(self.get_trip_descriptor(gtfs)),
            vehicle: Some(self.get_vehicle_descriptor()),
            position: Some(self.get_position()),
            current_stop_sequence: None,
            stop_id: None,
            current_status: None,
            timestamp: None,
            congestion_level: None,
            occupancy_status: None,
            occupancy_percentage: None,
            multi_carriage_details: vec![self.get_carriage_details()],
        }
    }
}

fn parse_data(data: String) -> Result<Vec<AnteaterExpressData>, Box<dyn Error + Send + Sync>> {
    let data: Vec<AnteaterExpressData> = from_str(&data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_dummy_gtfs() -> gtfs_structures::Gtfs {
        gtfs_structures::Gtfs::default()
    }

    fn create_example_string() -> String {
        String::from(
            r#"[{
                "GroundSpeed": 0,
                "Heading": 0,
                "IsDelayed": false,
                "IsOnRoute": true,
                "Latitude": 33.6502,
                "Longitude": -117.8428,
                "Name": "AE-02",
                "RouteID": 5,
                "Seconds": 3,
                "TimeStamp": "\/Date(1778635438000-0600)\/",
                "VehicleID": 3
            }, {
                "GroundSpeed": 0,
                "Heading": 0,
                "IsDelayed": false,
                "IsOnRoute": true,
                "Latitude": 33.6479,
                "Longitude": -117.835,
                "Name": "AE-05",
                "RouteID": 7,
                "Seconds": 2,
                "TimeStamp": "\/Date(1778635439000-0600)\/",
                "VehicleID": 6
            }, {
                "GroundSpeed": 0.10066213305,
                "Heading": 0,
                "IsDelayed": false,
                "IsOnRoute": true,
                "Latitude": 33.6504132,
                "Longitude": -117.824841,
                "Name": "AE-09",
                "RouteID": 6,
                "Seconds": 3,
                "TimeStamp": "\/Date(1778635438000-0600)\/",
                "VehicleID": 8
            }]
            "#,
        )
    }

    fn create_no_data_string() -> String {
        String::from("[]")
    }

    #[tokio::test]
    async fn gtfs_rt_from_live_data() {
        let actual_gtfs_data = Gtfs::from_url_async("https://ucirvine.transloc.com/Secure/Admin/Reports/GTFSDownload.aspx").await.unwrap();

        let rt_data = get_gtfs_rt(&actual_gtfs_data).await;
        assert!(rt_data.is_ok());

        println!("{:#?}", rt_data.unwrap());
    }

    #[test]
    fn gtfs_rt_from_string_no_data() {
        let example_data = create_no_data_string();
        let gtfs = get_dummy_gtfs();
        let anteater_gtfs = gtfs_rt_from_string(example_data, &gtfs);
        assert!(anteater_gtfs.is_ok());
    }

    #[test]
    fn gtfs_rt_from_string_is_ok() {
        let example_data = create_example_string();
        let gtfs = get_dummy_gtfs();
        let anteater_gtfs = gtfs_rt_from_string(example_data, &gtfs);
        assert!(anteater_gtfs.is_ok());
    }
}
