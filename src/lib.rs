use serde::Deserialize;
use serde_json::from_str;
use std::error::Error;
use std::time::SystemTime;
use gtfs_rt::*;
use gtfs_rt::vehicle_position::*;

pub async fn get_gtfs_rt() -> Result<gtfs_rt::FeedMessage, Box<dyn std::error::Error + Send + Sync>> {
    // steps:
    let data = reqwest::get("https://ucirvine.transloc.com/Services/JSONPRelay.svc/GetMapVehiclePoints?method=jQuery111104379215856036027_1712182850874&ApiKey=8882812681&_=1712182850877")
        .await?
        .text()
        .await?;
    return gtfs_rt_from_string(data);
}

fn gtfs_rt_from_string(data: String) -> Result<gtfs_rt::FeedMessage, Box<dyn std::error::Error + Send + Sync>> {
    let data = parse_data(data)?;
    let mut anteater_entities: Vec<FeedEntity> = Vec::new();
    for i in 0..data.len() {
        let vehicle = match data.get(i) {
            Some(x) => x,
            None => return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid String"
            ))),
        };
        anteater_entities.push(FeedEntity {
            id: i.to_string(),
            is_deleted: Some(false),
            trip_update: None,
            vehicle: Some(vehicle.get_vehicle_position()?),
            alert: None,
            shape: None,
        });
    }
    let anteater_gtfs = gtfs_rt::FeedMessage {
        header: FeedHeader {
            gtfs_realtime_version: "2.0".to_string(),
            incrementality: Some(1),
            timestamp: Some(SystemTime::now()
                .elapsed()?
                .as_secs()),
        },
        entity: anteater_entities.to_owned(),
    };
    return Ok(anteater_gtfs);
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
    route_id: i16,
    #[serde(rename = "VehicleID")]
    vehicle_id: i16,
}

impl AnteaterExpressData {

    fn get_carriage_details(&self) -> Result<CarriageDetails, Box<dyn Error + Send + Sync>> {
        return Ok(CarriageDetails {
            id: Some(self.vehicle_id.clone().to_string()),
            label: Some(self.vehicle_id.clone().to_string()),
            occupancy_status: None,
            occupancy_percentage: None,
            carriage_sequence: Some(1),
        });
    }

    fn get_position(&self) -> Result<Position, Box<dyn Error + Send + Sync>> {
        return Ok(Position {
            latitude: self.latitude,
            longitude: self.longitude,
            bearing: Some(self.heading),
            odometer: None,
            speed: Some(self.ground_speed),
        });
    }

    fn get_trip_descriptor(&self) -> Result<TripDescriptor, Box<dyn Error + Send + Sync>> {
        return Ok(TripDescriptor {
            trip_id: Some("".to_string()),
            route_id: Some(self.route_id.clone().to_string()),
            direction_id: Some(0),
            start_time: None,
            start_date: None,
            schedule_relationship: None,
        });
    }

    fn get_vehicle_descriptor(&self) -> Result<VehicleDescriptor, Box<dyn Error + Send + Sync>> {
        return Ok(VehicleDescriptor {
            id: Some(self.vehicle_id.to_string().clone()),
            label: Some(self.name.clone()),
            license_plate: None,
            wheelchair_accessible: None,
        })
    }

    fn get_vehicle_position(&self) -> Result<VehiclePosition, Box<dyn Error + Send + Sync>> {
        return Ok(VehiclePosition {
            trip: Some(self.get_trip_descriptor()?),
            vehicle: Some(self.get_vehicle_descriptor()?),
            position: Some(self.get_position()?),
            current_stop_sequence: None, //fetch from gtfs
            stop_id: None, //fetch from gtfs
            current_status: None,
            timestamp: None,
            congestion_level: None,
            occupancy_status: None,
            occupancy_percentage: None,
            multi_carriage_details: vec![self.get_carriage_details()?],
        });
    }
}

/*
Parses the AnteaterExpress feed, in the form of a string, 
into a struct, or returns an error if the string provided 
is invalid.
 */
fn parse_data(data: String) -> Result<Vec<AnteaterExpressData>, Box<dyn Error + Send + Sync>> {
    let prefix_index = data.find("("); 
    let suffix_index = data.chars().rev().position(|x| x == ')');
    if let (Some(prefix_index), Some(suffix_index)) = (prefix_index, suffix_index) {
        //index at which data starts
        let suffix_index = data.len() - (suffix_index + 1);
        //index at which data ends
        let prefix_index = prefix_index + 1;
        // data is iterated through, skips up to prefix_index, takes up to suffix_index, and collects the values
        let data: String = data
            .chars()
            .skip(prefix_index)
            .take(suffix_index - prefix_index)
            .collect();
        let data: Vec<AnteaterExpressData> = from_str(&data)?;
        Ok(data)
    } else {
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Invalid String",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_data_is_ok() {
        let example_data = String::from(
            r#"jQuery111104379215856036027_1712182850874( 
            [
                {
                    "GroundSpeed":10.99901573793,
                    "Heading":78,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.64704,
                    "Longitude":-117.82938,
                    "Name":"AE02",
                    "RouteID":4,
                    "Seconds":3,
                    "TimeStamp":"\/Date(1712229203000-0600)\/",
                    "VehicleID":3
                },
                {
                    "GroundSpeed":18.79921258116,
                    "Heading":289,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.6478503,
                    "Longitude":-117.8260522,
                    "Name":"AE04",
                    "RouteID":4,
                    "Seconds":2,
                    "TimeStamp":"\/Date(1712229204000-0600)\/",
                    "VehicleID":5
                },
                {
                    "GroundSpeed":0.10066213305,
                    "Heading":0,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.6489682,
                    "Longitude":-117.839556,
                    "Name":"AE11",
                    "RouteID":4,
                    "Seconds":2,
                    "TimeStamp":"\/Date(1712229204000-0600)\/",
                    "VehicleID":10
                },
                {
                    "GroundSpeed":0,
                    "Heading":0,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.6489278979,
                    "Longitude":-117.8448493721,
                    "Name":"AE08",
                    "RouteID":5,
                    "Seconds":2,
                    "TimeStamp":"\/Date(1712229204000-0600)\/",
                    "VehicleID":7
                }
            ] 
        );"#,
        );
        let deserialized_data = parse_data(example_data);
        assert!(deserialized_data.is_ok());
    }

    #[test]
    fn parse_data_is_correct() {
        let example_data = String::from(
            r#"jQuery111104379215856036027_1712182850874( 
            [
                {
                    "GroundSpeed":10.99901573793,
                    "Heading":78,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.64704,
                    "Longitude":-117.82938,
                    "Name":"AE02",
                    "RouteID":4,
                    "Seconds":3,
                    "TimeStamp":"\/Date(1712229203000-0600)\/",
                    "VehicleID":3
                },
                {
                    "GroundSpeed":18.79921258116,
                    "Heading":289,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.6478503,
                    "Longitude":-117.8260522,
                    "Name":"AE04",
                    "RouteID":4,
                    "Seconds":2,
                    "TimeStamp":"\/Date(1712229204000-0600)\/",
                    "VehicleID":5
                },
                {
                    "GroundSpeed":0.10066213305,
                    "Heading":0,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.6489682,
                    "Longitude":-117.839556,
                    "Name":"AE11",
                    "RouteID":4,
                    "Seconds":2,
                    "TimeStamp":"\/Date(1712229204000-0600)\/",
                    "VehicleID":10
                },
                {
                    "GroundSpeed":0,
                    "Heading":0,
                    "IsDelayed":false,
                    "IsOnRoute":true,
                    "Latitude":33.6489278979,
                    "Longitude":-117.8448493721,
                    "Name":"AE08",
                    "RouteID":5,
                    "Seconds":2,
                    "TimeStamp":"\/Date(1712229204000-0600)\/",
                    "VehicleID":7
                }
            ] 
        );"#,
        );
        let deserialized_data = match parse_data(example_data) {
            Ok(x) => x,
            _ => return,
        };
        let vehicle_0 = match deserialized_data.get(0) {
            Some(x) => x,
            _ => return,
        };
        let vehicle_2 = match deserialized_data.get(2) {
            Some(x) => x,
            _ => return,
        };

        assert_eq!(vehicle_0.ground_speed, 10.99901573793);
        assert_eq!(vehicle_2.vehicle_id, 10);

    }
}
