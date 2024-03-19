use serde::Deserialize;

/// Light message generated from another system.
#[derive(Deserialize, Debug, PartialEq)]
pub struct LightMessage {
    /// The channels of the PDM to turn on.
    /// TODO: The lighting system was created on the fly when the machine got to
    /// to the farm as electrical did not want to hard wire it in the short time
    /// frame (and lack of switch on site). As a result the channels that the
    /// lights have been connected to are not matched (i.e. crop be 0 - to channel 1)
    pub channels: Vec<u8>,
    /// If true, set the PWM of the output channel to 100, else to 0.
    pub is_on: bool,
    /// Camera id associated with the light.
    cam_id: u8,
    /// Crop bed id associated with the light.
    crop_bed_id: u8,
}

#[cfg(test)]
mod tests {

    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(
        r#"{"channels": [7, 8, 9], 
        "is_on": true,
               "cam_id": 5, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [0],
        "is_on": true,
                "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [1],
        "is_on": true,
                   "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [2],
        "is_on": true,
                   "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [3],
        "is_on": true,
                   "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [4],
        "is_on": true,
                   "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [5],
        "is_on": true,
                   "cam_id": 4,
                   "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [6],
        "is_on": true,
                    "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [7],
        "is_on": true,
                    "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [8],
        "is_on": true,
                    "cam_id": 4, "crop_bed_id": 2}"#
    )]
    #[case(
        r#"{"channels": [9],
        "is_on": true,
                    "cam_id": 4, "crop_bed_id": 2}"#
    )]
    fn test_parse_weed_message(#[case] raw_string: &str) {
        let _parsed: LightMessage = serde_json::from_str(raw_string).unwrap();
    }

    #[rstest]
    #[case((
        r#"{"channels": [7, 8, 9], "is_on": false,
               "cam_id": 5, "crop_bed_id": 2}"#
    , LightMessage {
            cam_id: 5,
            is_on: false,
            crop_bed_id: 2,
            channels: vec![7, 8, 9],

        }))]
    #[case((
        r#"{"channels": [0], "is_on": true,
                "cam_id": 4, "crop_bed_id": 2}"#
    , LightMessage {
            cam_id: 4,
            is_on: true,
            crop_bed_id: 2,
            channels: vec![0],
        } ))]
    fn test_parse_and_compare_weed_message(#[case] args: (&str, LightMessage)) {
        let parsed: LightMessage = serde_json::from_str(args.0).unwrap();

        assert_eq!(parsed, args.1, "Failed to parse message correctly");
    }
}
