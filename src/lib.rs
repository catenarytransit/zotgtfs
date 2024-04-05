use serde::Deserialize;
use serde_json::from_str;
use std::error::Error;
use std::time::SystemTime;
use gtfs_rt::*;

pub async fn get_gtfs_rt() -> Result<gtfs_rt::FeedMessage, Box<dyn std::error::Error + Send + Sync>> {
    // steps:
    // let anteater_data = parse_data("[website information]")?;
    let anteater_entities: Vec<FeedEntity> = Vec::new();
    // for i in 1..anteater_data.len() {
    //      let vehicle = match deserialized_data.get(i) {
    //          Some(x) => x,
    //          _ => Err(Box::new(std::io::Error::new(
    //              std::io::ErrorKind::Other,
    //              "Invalid String",
    //              ))),
    //      };
    //      anteater_entities.push(FeedEntity {
    //          id: i,
    //          is_deleted: false,
    //          trip_update: None,
    //          vehicle: VehiclePosition {
    //              trip: TripDescriptor {
    //                  trip_id: "", //fetch from gtfs static
    //                  route_id: vehicle.route_id,
    //                  direction_id: 0,
    //                  start_time: None,
    //                  start_date: None,
    //                  schedule_relationship: None,
    //                  modified_trip: None,
    //              },
    //              vehicle: VehicleDescriptor {
    //                  id: vehicle.vehicle_id,
    //                  label: vehicle.name,
    //                  license_plate: None,
    //                  wheelchair_accessible: None,
    //              },
    //              position: Position {
    //                  latitude: vehicle.latitude,
    //                  logitude: vehicle.longitude,
    //                  bearing: vehicle.heading,
    //                  odometer: None,
    //                  speed: GroundSpeed,
    //              },
    //              current_stop_sequence: 0, //fetch from gtfs
    //              stop_id: "", //fetch from gtfs
    //              current_status: None,
    //              timestamp: None,
    //              congestion_level: None,
    //              occupancy_status: None,
    //              occupancy_percentage: None,
    //              multi_carriage_details: None,
    //          },
    //          alert: None,
    //          shape: None,
    //          stop: None, //fetch from gtfs
    //          trip_modifications: None,
    //      });
    // }
    let anteater_gtfs = gtfs_rt::FeedMessage {
        header: FeedHeader {
            gtfs_realtime_version: "2.0".to_string(),
            incrementality: Some(1),
            timestamp: Some(SystemTime::now()
                .elapsed()?
                .as_secs()),
        },
        entity: anteater_entities,
    };
    return Ok(anteater_gtfs);
}

#[derive(Deserialize)]
struct AnteaterExpressData {
    #[serde(rename = "GroundSpeed")]
    ground_speed: f64,
    #[serde(rename = "Heading")]
    heading: f64,
    #[serde(rename = "IsDelayed")]
    is_delayed: bool,
    #[serde(rename = "IsOnRoute")]
    is_on_route: bool,
    #[serde(rename = "Latitude")]
    latitude: f64,
    #[serde(rename = "Longitude")]
    longitude: f64,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "RouteID")]
    route_id: i16,
    #[serde(rename = "Seconds")]
    seconds: i32,
    #[serde(rename = "TimeStamp")]
    time_stamp: String,
    #[serde(rename = "VehicleID")]
    vehicle_id: i16,
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
        assert_eq!(vehicle_2.is_delayed, false);

    }
}
