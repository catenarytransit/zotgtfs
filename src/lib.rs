use serde::Deserialize;
use serde_json::from_str;
use std::error::Error;

// pub struct AntExGtfsRt {
//     pub vehicle_positions: gtfs_rt::FeedMessage,
//     //pub trip_positions: gtfs_rt::FeedMessage,
// }
// pub async fn get_gtfs_rt() -> Result<AntExGtfsRt, Box<dyn Error + Send + Sync>> {

// }

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

fn parse_data(data: String) -> Result<Vec<AnteaterExpressData>, Box<dyn Error + Send + Sync>> {
    let prefix_index = data.find("(");
    let suffix_index = data.chars().rev().position(|x| x == ')');
    if let (Some(prefix_index), Some(suffix_index)) = (prefix_index, suffix_index) {
        let suffix_index = data.len() - (suffix_index + 1);
        let prefix_index = prefix_index + 1;
        println!("{}, {}", prefix_index, suffix_index);
        let data: String = data
            .chars()
            .skip(prefix_index)
            .take(suffix_index - prefix_index)
            .collect();
        println!("{}", data);
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
    fn test_parse_data() {
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
}
