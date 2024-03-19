use ix3212_pdm::{pdm::Pdm as PdmDriver, prelude::*};
use serde::{Deserialize, Serialize, Serializer};
use socketcan::tokio::CanSocket as AsyncCanSocket;
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    path::Path,
    sync::Arc,
};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Similar to the camera, a PDM (power delivery module) is created
/// using the builder pattern that consumes a PDM configuration. A
/// PDM config is used for one unit. Generally a crop bed will use
/// two PDMs to ensure coverage for all the actuated solenoids.
#[derive(Deserialize, Serialize, PartialEq, Eq, Debug)]
pub struct PdmConfig {
    /// Source address of the PDM on the canbus network. For the ix-3212
    /// the address can only be changed by physically altering the wire
    /// states on the PDM pin out (four total).
    // TODO: Implement the four address states as a separate enum in the
    // ix-3212 crate.
    pub address: u8,
    /// Location in terms of bill of materials.
    bed_location_id: u8,
    /// PDM Function Config, see technical specification for ix-3212
    #[serde(serialize_with = "ordered_u8_map")]
    output_function_config: HashMap<u8, OutputFunctionConfigPayload>,
    /// PDM Channel Config, see technical specification for ix-3212
    #[serde(serialize_with = "ordered_u8_map")]
    output_channels_config: HashMap<u8, ChannelConfig>,
}
/// Orders the channel configuration in the yaml file.
/// if this mapping is not used there is no guarantee
/// that the config files will not be over written
/// needlessly (i.e. same information in different order)
///
/// * `value`: `HashMap`
/// * `serializer`: Serializer
// TODO: extract this to utils
pub fn ordered_u8_map<S, T>(value: &HashMap<u8, T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    let ordered: BTreeMap<_, _> = value.iter().collect();
    ordered.serialize(serializer)
}

impl PdmConfig {
    /// Create a new PDM config without function, or channel
    /// configurations. For more information on configuration
    /// please review the ix-3212 crate, and the technical
    /// specification provided by the manufacturer.
    ///
    /// * `address`: address of the PDM.
    /// * `bed_location_id`: identify unit in-line with bill of materials.
    pub fn new(address: u8, bed_location_id: u8) -> Self {
        Self {
            address,
            bed_location_id,
            output_function_config: HashMap::new(),
            output_channels_config: HashMap::new(),
        }
    }

    /// Create a `PdmConfig` by reading data from a file.
    ///
    /// * `filepath`: Path to file with configuration parameters.
    pub fn from_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        let file = Path::new(&filepath);
        let pdm_config = if file.is_file() {
            let config_file = config::Config::builder()
                .add_source(config::File::new(
                    &file.to_string_lossy(),
                    config::FileFormat::Yaml,
                ))
                .build()
                .expect("Failed read config");

            config_file
                .try_deserialize::<Self>()
                .expect("Failed to parse config file into struct")
        } else {
            panic!("Could not locate the config file {:?}", file);
        };
        pdm_config
    }
}

/// Similar to the `OnyxCamera` provide a wrapper struct type
/// that provides access to the underlying driver that can
/// be configured by consuming a `PdmConfig` in the builder
/// pattern.
// TODO: Consistent naming, move this to OnyxPdm, and rename
//       the PdmDriver import crate as PDM etc. Good first 
//       issue.
#[allow(dead_code)]
pub struct Pdm {
    /// Unique identifier for a Pdm in the system.
    uuid: Uuid,
    /// Control calls to developed pdm driver. More information
    /// can be found in the ['Pdm'] implementation.
    pub driver: PdmDriver,
    /// Config used to create the Pdm.
    config: PdmConfig,
    /// Location in the bed for the Pdm.
    /// TODO: Change this to a location enum.
    bed_location_id: u8,
}

impl Pdm {
    /// Create a new onyx Pdm by consuming a `PdmConfig`.
    ///
    /// * `config`: Set of config parameters.
    pub fn new(config: PdmConfig) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4(),
            bed_location_id: config.bed_location_id,
            driver: PdmDriver::new(config.address),
            config,
        }
    }

    /// Initialise the PDM with the configuration files passed to
    /// [`Pdm::new(config`: `PdmConfig`]. Registering an interface in
    /// this manner enables the component to manage how PDMs can
    /// be connected to which canbus trunk line (interface), i.e.
    /// can0, can1. Note: Pdm's must be configured and confirmed
    /// to be in the right configuration prior to sending messages.
    // TODO: Pass by reference not mutable.
    // TODO: Pass by reference for configure output calls.
    // TODO: Implement periodic configuration checks during runtime.
    pub async fn initialise(&mut self, interface: Arc<Mutex<AsyncCanSocket>>) {
        // set the PDM to use the correct interface.
        self.driver.set_interface(interface);

        self.driver
            .configure_output_function(self.config.output_function_config.clone())
            .await;
        self.driver
            .configure_output_channels(self.config.output_channels_config.clone())
            .await;
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(30, 0)]
    #[case(31, 1)]
    fn test_read_write_pdm_to_config_file(#[case] pdm_address: u8, #[case] bed_location_id: u8) {
        let mut write_config = PdmConfig::new(pdm_address, bed_location_id);

        for channel_number in 1u8..=12u8 {
            write_config.output_function_config.insert(
                channel_number,
                OutputFunctionConfigPayload::new()
                    .with_channel(ChannelNumber::new(channel_number))
                    .with_load_profile(LoadProfile::Lamp)
                    .with_loss_of_communication(LossOfCommunication::CHZero)
                    .with_soft_start_step_size(SoftStartStepSize::new(None, false))
                    .with_local_source_control(
                        LocalSourceControl::new()
                            .with_calibration_time(LocalSourceCalibration::Unsupported)
                            .with_input(DigitalInputChannel::new(None, false))
                            .with_response(LocalSourceControlResponse::ActiveLowHigh),
                    )
                    .with_power_on_reset(
                        PowerOnReset::new()
                            .with_loss_of_can_feature_enabled(true)
                            .with_enable(false)
                            .with_motor_braking(MotorBraking::Disabled)
                            .with_command(PowerOnResetCommand::new(0.00)),
                    ),
            );
        }

        for channel_number in 1u8..=12u8 {
            write_config.output_channels_config.insert(
                channel_number,
                ChannelConfig::new()
                    .with_channel_load_control(ChannelLoadControl::HighSide)
                    .with_feeadback_type(FeedbackType::Current)
                    .with_current_limit(CurentLimit {
                        limit: 5.0,
                        reserved: false,
                    })
                    .with_automatic_reset(true),
            );
        }

        let write_file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(format!(
                "{}/config/devices/crop_bed/pdm_{bed_location_id}.yaml",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("Couldn't open file");

        serde_yaml::to_writer(write_file, &write_config).unwrap();

        let test_file = std::fs::File::open(format!(
            "{}/config/devices/crop_bed/pdm_{bed_location_id}.yaml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("Could not open file.");

        let read_config: PdmConfig =
            serde_yaml::from_reader(&test_file).expect("Could not read values.");

        assert_eq!(write_config, read_config, "Failed to be created equally");
    }

    #[rstest]
    #[case(30, 0)]
    fn test_read_write_utilities_pdm_to_config_file(
        #[case] pdm_address: u8,
        #[case] bed_location_id: u8,
    ) {
        let mut write_config = PdmConfig::new(pdm_address, bed_location_id);

        for channel_number in 1u8..=12u8 {
            write_config.output_function_config.insert(
                channel_number,
                OutputFunctionConfigPayload::new()
                    .with_channel(ChannelNumber::new(channel_number))
                    .with_load_profile(LoadProfile::Lamp)
                    .with_loss_of_communication(LossOfCommunication::CHZero)
                    .with_soft_start_step_size(SoftStartStepSize::new(None, false))
                    .with_local_source_control(
                        LocalSourceControl::new()
                            .with_calibration_time(LocalSourceCalibration::Unsupported)
                            .with_input(DigitalInputChannel::new(None, false))
                            .with_response(LocalSourceControlResponse::ActiveLowHigh),
                    )
                    .with_power_on_reset(
                        PowerOnReset::new()
                            .with_loss_of_can_feature_enabled(true)
                            .with_enable(false)
                            .with_motor_braking(MotorBraking::Disabled)
                            .with_command(PowerOnResetCommand::new(0.00)),
                    ),
            );
        }

        for channel_number in 1u8..=12u8 {
            write_config.output_channels_config.insert(
                channel_number,
                ChannelConfig::new()
                    .with_channel_load_control(ChannelLoadControl::HighSide)
                    .with_feeadback_type(FeedbackType::Current)
                    .with_current_limit(CurentLimit {
                        limit: 15.0,
                        reserved: false,
                    })
                    .with_automatic_reset(true),
            );
        }

        let write_file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(format!(
                "{}/config/devices/crop_bed/pdm_utilities.yaml",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("Couldn't open file");

        serde_yaml::to_writer(write_file, &write_config).unwrap();

        let test_file = std::fs::File::open(format!(
            "{}/config/devices/crop_bed/pdm_utilities.yaml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("Could not open file.");

        let read_config: PdmConfig =
            serde_yaml::from_reader(&test_file).expect("Could not read values.");

        assert_eq!(write_config, read_config, "Failed to be created equally");
    }
}
