use crate::utils::image::{CameraPixelFormat, Roi};
use aravis::{AcquisitionMode, Camera, CameraExt, CameraExtManual, StreamExt};
use chrono::{DateTime, Utc};
use image::DynamicImage;
use serde::{de::Visitor, Deserialize, Serialize};
use std::{
    ffi::OsStr,
    net::Ipv4Addr,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc, Barrier,
    },
    time::{Duration, Instant},
};
use strum_macros::{EnumString, IntoStaticStr};
use uuid::Uuid;

/// You can trigger the device in several ways as per the
/// genicam standard, however for the onyx use case only
/// the software trigger was implemented.
#[derive(EnumString, Deserialize, Serialize, IntoStaticStr, Copy, Clone, Debug, PartialEq, Eq)]
pub enum DeviceTrigger {
    /// Software available trigger.
    Software,
}

/// Due to rusts orphan rule at times we need to provide wrapper types for struct's
/// that come from other crates. The convention used in this software is to lead with
/// `WrapperNameOfType`. This is seen a lot with the serde crate.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct WrapperAcquisitionMode(AcquisitionMode);

impl Serialize for WrapperAcquisitionMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.0 {
            AcquisitionMode::Continuous => {
                serializer.serialize_unit_variant("AcquisitionMode", 0, "Continuous")
            }
            AcquisitionMode::SingleFrame => {
                serializer.serialize_unit_variant("AcquisitionMode", 1, "SingleFrame")
            }
            AcquisitionMode::MultiFrame => {
                serializer.serialize_unit_variant("AcquisitionMode", 2, "MultiFrame")
            }
            _ => panic!("Unknown acquisition mode"),
        }
    }
}

impl<'de> Deserialize<'de> for WrapperAcquisitionMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(CameraAcquisitionModeVisitor {})
    }
}

/// When implementing serde types we need to provide a `Visitor` type which is used
/// for the implementation of the Visitor trait. See the [serde] crate for more
/// information.
struct CameraAcquisitionModeVisitor {}

impl<'de> Visitor<'de> for CameraAcquisitionModeVisitor {
    type Value = WrapperAcquisitionMode;

    // TODO: Update function to elided lifetime below. Good first issue.
    #[allow(unused_attributes)]
    #[allow(elided_lifetimes_in_paths)]
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Could not deserialise CameraAcquisitionMode")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match v {
            "Continuous" => Ok(WrapperAcquisitionMode(AcquisitionMode::Continuous)),
            "SingleFrame" => Ok(WrapperAcquisitionMode(AcquisitionMode::SingleFrame)),
            _ => Err(serde::de::Error::custom(
                "Unknown acquisition mode format {v:?}",
            )),
        }
    }
}

/// Camera configuration struct contains all of the above specified parameters
/// that interface with the genicam standard, and the aravis camera driver.
#[derive(Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct OnyxCameraConfig {
    /// Location of the device on the crop bed as per bill of materials.
    bed_location_id: Option<u8>,
    /// Frames per second specified in Hz
    fps: u32,
    /// Network address of the camera.
    ip_address: Ipv4Addr,
    /// Region of interest (ROI), used for cropping the total image from the camera.
    roi: Option<Roi>,
    /// Different cameras provide different pixel compression formats.
    pixel_format: Option<CameraPixelFormat>,
    /// The type of trigger to set for the camera to capture an image.
    trigger: Option<DeviceTrigger>,
    /// Acquisition mode determines how the images are captured such as continuous or single frame.
    acquisition_mode: Option<WrapperAcquisitionMode>,
    /// A cameras ability to send data over a network is impacted by the MTU size, this setting automatically
    /// determines the maximum MTU that the camera can apply.
    auto_packet_size: Option<bool>,
    /// Allow auto gain settings for the device.
    auto_gain: Option<bool>,
    /// Allow auto brightness settings for the device.
    auto_brightness: Option<bool>,
    /// Allow auto exposure settings for the device.
    auto_exposure: Option<bool>,
    /// Exposure min limit bounds in microseconds.
    exposure_min: Option<i32>,
    /// Exposure max limit bounds in microseconds.
    exposure_max: Option<i32>,
}

impl OnyxCameraConfig {
    /// Create new camera configuration using defaults.
    ///
    /// * `ip_address`: IP address of networked camera.
    /// * `fps`: desired frame per second for capture.
    pub fn new(ip_address: impl Into<Ipv4Addr>, fps: u32) -> Self {
        Self {
            fps,
            ip_address: ip_address.into(),
            roi: Default::default(),
            pixel_format: Default::default(),
            trigger: Default::default(),
            acquisition_mode: Default::default(),
            auto_packet_size: Default::default(),
            bed_location_id: Default::default(),
            auto_gain: Default::default(),
            auto_exposure: Default::default(),
            auto_brightness: Default::default(),
            exposure_min: Default::default(),
            exposure_max: Default::default(),
        }
    }

    /// Generates a new camera config from a file.
    ///
    /// * `filepath`: path to config file.
    pub fn from_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        let file = Path::new(&filepath);
        let camera_config = if file.is_file() {
            let config_file = config::Config::builder()
                .add_source(config::File::new(
                    &file.to_string_lossy(),
                    config::FileFormat::Yaml,
                ))
                .build()
                .expect("Failed read config");
            config_file
                .try_deserialize::<OnyxCameraConfig>()
                .expect("Failed to parse config file into struct")
        } else {
            panic!("Could not locate the config file {:?}", file);
        };
        camera_config
    }
}

/// The general method for integrating a new device into the onyx system is to
/// give each item a specific UUID (for logging, telemetry, trouble shooting.)
/// and allow a public interface to an underlying driver. This driver is either
/// implemented by flux, such as the IX3212 PDM, or relies on an open source
/// or manufacture provided driver, such as aravis (open source).
pub struct OnyxCamera {
    /// Access to the aravis driver for camera functionality.
    pub driver: Camera,
    /// Unique identifier, helpful for trouble shooting and logging.
    uuid: Uuid,
    /// Location of the device on the crop bed as per bill of materials.
    bed_location_id: Option<u8>,
}

// TODO: extract out common functionality to traits. Didn't get time to do a
// refactor and pull these out due to delivery constraints. In addition async
// functions in traits where still fuzzy.
impl OnyxCamera {

    /// Return the unique identifier of the camera.
    pub fn get_uuid(&self) -> Uuid {
        self.uuid
    }

    /// Set an id for for where in the module this particular device is
    /// housed. Advise to keep in communication with Electrical, Mechanical
    /// engineering to ensure consistent nomenclature.
    ///
    /// * `location_id`: integer number that can be linked to a bill of materials (BOM)
    pub fn set_location_id(&mut self, location_id: u8) {
        self.bed_location_id = Some(location_id);
    }

    /// Create a new Onyx Camera by consuming a camera config.
    ///
    /// * `config`: Set of parameters that configure a network camera.
    pub fn new(config: OnyxCameraConfig) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            bed_location_id: config.bed_location_id,
            driver: Self::build_from_config(config),
        }
    }

    /// Create a new Onyx Camera by reading a file at a location 
    /// parsing it as a `OnyxCameraConfig` and consuming that 
    /// config as per the builder patter.
    ///
    /// * `filepath`: path to the parameter file.
    pub fn from_config_file<F: AsRef<OsStr>>(filepath: F) -> Self {
        Self::new(OnyxCameraConfig::from_file(filepath))
    }

    /// Create an aravis camera handle for the `OnyxCamera` driver. Due to the way
    /// genicam works there can be issues with the order in which certain camera
    /// properties are set (it follows a graph approach). This can be frustrating
    /// to troubleshoot as a camera data sheet will specify a certain capability,
    /// but may not work given the order of configuration steps. If this happens
    /// the recommendation is to write additional unit tests below.
    ///
    /// * `config`: `OnyxCamera` config struct
    fn build_from_config(config: OnyxCameraConfig) -> Camera {
        // TODO:
        // As more camera tuning was required when getting the unit onto the customers farm
        // additional parameters were implemented and patched on, the result of this
        // is this very long function. However none of the individual checks get
        // called again so pulling them out didn't make sense. Additional logic needs to be
        // implemented for camera recovery (i.e. when there is a loose Ethernet), it would make
        // sense to look at this in tandem with that activity.
        let camera: Camera = match Camera::new(Some(&config.ip_address.to_string())) {
            Ok(c) => c,
            Err(e) => panic!("Failed to create camera {e:?}"),
        };

        // Some cameras will fail silently if you try to put a higher FPS in
        // that can be tolerated by the device. I don't believe genicam (xml)
        // will stop you putting an erroneous value in. TODO: review the
        // aravis repo and check the wrapper functions in there.
        match camera.frame_rate_bounds() {
            Ok((min, max)) => {
                assert!(
                    (min..max).contains(&config.fps.into()),
                    "Cannot set FPS as device range does not allow it"
                );
            }
            Err(e) => panic!("Cannot determine frame rate bounds; {e}"),
        }

        //TODO: refactor this into above match statement.
        if let Err(e) = camera.set_frame_rate(config.fps.into()) {
            panic!("Failed to set frame rate {e:?}")
        }

        // Setting the region of interest requires some effort depending on if
        // the sensor is utilising binning. See camera data sheet or Gig E vision
        // specification to learn more.
        if let Some(roi) = config.roi {
            if let Ok(binning_available) = camera.is_binning_available() {
                if binning_available {
                    if let Ok((min_y, max_y)) = camera.y_binning_bounds() {
                        for y in (2..=max_y).step_by(2) {
                            assert!(
                                roi.h % y == 0,
                                "ROI is not a muliple of the binning bounds in the Y direction bounds {:?}, y: {}", (min_y, max_y), y
                            );
                        }
                    } else {
                        panic!("Cannot automatically determine X direction binning bounds for the camera")
                    }

                    if let Ok((min_x, max_x)) = camera.x_binning_bounds() {
                        for x in (2..=max_x).step_by(2) {
                            assert!(
                                roi.x % x == 0,
                                "ROI is not a muliple of the binning bounds in the X direction bounds {:?}, x: {}", (min_x, max_x), x
                            );
                        }
                    } else {
                        panic!("Cannot automatically determine Y direction binning bounds for the camera")
                    }
                }
            }
            if let Err(e) = camera.set_region(roi.x, roi.y, roi.w, roi.h) {
                panic!("Failed to set acquisition roi {e:?}")
            }

            if let Ok((x, y, w, h)) = camera.region() {
                assert!(x == roi.x, "Failed initialisation assert to set offset x");
                assert!(y == roi.y, "Failed initialisation assert to set offset y");
                assert!(w == roi.w, "Failed initialisation assert to set width  w");
                assert!(h == roi.h, "Failed initialisation assert to set height h");
            }
        }

        if let Some(pixel_format) = config.pixel_format {
            if let Err(e) = camera.set_pixel_format(pixel_format.0) {
                panic!("Failed to set pixel format {e:?}")
            }
        }

        if let Some(acquisition_mode) = config.acquisition_mode {
            if let Err(e) = camera.set_acquisition_mode(acquisition_mode.0) {
                panic!("Failed to set acquisition mode {e:?}")
            }
        }

        if let Some(auto_exposure) = config.auto_exposure {
            if let Ok(available) = camera.is_exposure_auto_available() {
                if available {
                    if auto_exposure {
                        if let Err(e) = camera.set_exposure_time_auto(aravis::Auto::Continuous) {
                            panic!("Failed to set exposure time auto {e}");
                        }
                    }
                } else {
                    println!("Auto Exposure is not available");
                }
            }
        }

        if let Some(auto_brightness) = config.auto_brightness {
            if auto_brightness {
                if let Err(e) = camera.set_string("autoBrightnessMode", "Active") {
                    panic!("Failed to set auto auto brightness {e}")
                }
            }
        }

        if let Some(exposure_min) = config.exposure_min {
            if let Err(e) = camera.set_float("exposureAutoMinValue", exposure_min as f64) {
                panic!("Failed to set auto min time {e}");
            }
        }
        // TODO: Set logging to tell when exposure max goes above 10,000
        if let Some(exposure_max) = config.exposure_max {
            if let Err(e) = camera.set_float("exposureAutoMaxValue", exposure_max as f64) {
                panic!("Failed to set auto min time {e}");
            }
        }

        if let Some(auto_gain) = config.auto_gain {
            if let Ok(available) = camera.is_gain_auto_available() {
                if available {
                    if auto_gain {
                        if let Err(e) = camera.gain_auto() {
                            panic!("Failed to set auto gain {e}");
                        }
                    }
                } else {
                    println!("Auto gane is not available");
                }
            }
        }

        // TODO: Create some config enums for this. Good first issue.
        //       and refrain from having &str config without type safety.
        if let Err(e) = camera.set_string("BalanceWhiteAuto", "OnDemand") {
            panic!("Failed to set on demand white balance {e}");
        }
        // Need to set this last so we do not overwrite the configurations.
        if let Some(trigger) = config.trigger {
            if let Err(e) = camera.set_trigger(trigger.into()) {
                panic!("Failed to set acquisition mode {e:?}")
            }
        }

        if let Some(auto_packet_size) = config.auto_packet_size {
            if auto_packet_size {
                if let Err(e) = camera.gv_auto_packet_size() {
                    panic!("Failed to set auto streaming packet size (MTU) {e:?}")
                }
            }
        }
        camera
    }
}

/// Helper function to create the buffer that is filled by the camera when
/// it is triggered. We create a closure to allow us to wrap the generation
/// process with the region of interest (ROI) specifications that are required
/// in the onyx system.
fn make_buffer_closure(camera: &OnyxCamera) -> impl Fn() -> aravis::Buffer {
    let (_, _, w, h) = camera.driver.region().expect("Failed to get buffer area");
    let pixel_format = camera
        .driver
        .pixel_format()
        .expect("Failed to get pixel format");

    //TODO: Look at the use of the offsets and what they actually
    // pertain to from the genicam standards. I believe it is a 
    // width and hight from the top left pixel location for the 
    // ROI.

    #[allow(clippy::cast_sign_loss)]
    // SAFETY: w and h should not be negative numbers anyway, could look into
    // changing the data type for the serialisation format to a usize anyway.
    move || aravis::Buffer::new_leaked_image(pixel_format, w as usize, h as usize)
}

/// Device payloads contain data and information that is passed from a
/// Device up to the parent component using MPSC channels. In the case
/// of the onyx camera its the information from the image sensor and
/// the exact time of capture.
#[allow(dead_code)]
pub struct DevicePayload {
    /// Unique identifier for the payload event.
    uuid: Uuid,
    /// Matrix of pixel values from the camera taken during software trigger.
    pub image: DynamicImage,
    /// Image capture time.
    datetime: DateTime<Utc>,
    /// Location of device that took the image.
    location_id: Option<u8>,
}

impl DevicePayload {
    /// Generate a filename for the image generated from a specific
    /// `OnyxCamera` device.
    // TODO: Find open source image pipe library to eradicate needless
    //       writes to disk. Didn't have time to implement or adapt AI
    //       system before on farm delivery.
    pub fn filename(&self) -> String {
        if let Some(ref location_id) = self.location_id {
            format!("{}/{}.png", location_id, self.datetime)
        } else {
            format!("{}.png", self.datetime)
        }
    }
}

/// A camera controller unit struct is used to group the 
/// device actions together so that it can be accessed by 
/// the component.
pub struct CameraController;

impl CameraController {
    /// Start streaming images from the camera and sending the payload
    /// back up to the parent component. TODO: Look into soft restart
    /// recovery from failure.
    ///
    /// * `camera`: an onyx camera device
    /// * `stop_signal`: Will halt the camera streaming.
    /// * `barrier`: Linked thread barrier for other camera devices.
    /// * `image_channel`: MPSC channel for sharing payloads.
    pub fn start(
        camera: OnyxCamera,
        stop_signal: Arc<AtomicBool>,
        barrier: Arc<Barrier>,
        image_channel: Sender<DevicePayload>,
    ) {
        let uuid = camera.uuid;
        let build_buffer = make_buffer_closure(&camera);
        let interval_ms = Duration::from_secs_f64(
            1.0 / camera
                .driver
                .frame_rate()
                .expect("Failed to get frame rate"),
        )
        .as_millis();

        let camera_stream = camera
            .driver
            .create_stream()
            .expect("Unable to create camera stream");

        camera_stream.push_buffer(&build_buffer());

        camera
            .driver
            .start_acquisition()
            .expect("Unable to start camera acquisition");

        // Some cameras don't have auto white balance, or auto gain etc.
        // so they have to be manually implemented during the camera capture
        // hot loop. Several of these were found during on customers farm in
        // first whole system test. TODO: Add this field into the device
        // config struct.
        let config_limit = 5;
        let mut config_tick = Instant::now();

        // Wait for all threads, no re sync is implemented yet.
        // TODO: Review sync primitives to asses drift between
        // cameras. May be more involved if you are also going 
        // to sync the light actuation system.
        barrier.wait();
        while !stop_signal.load(Ordering::Relaxed) {
            let tick = Instant::now();

            // Take care of non auto based camera properties.
            // TODO: There are several of this &str's in the
            //       genicam spec, remove them to there own
            //       crate or module.
            if config_tick.elapsed().as_secs() > config_limit {
                if let Err(e) = camera.driver.execute_command("balanceWhiteAutoOnDemandCmd") {
                    panic!("Failed to call white balance {e}")
                }
                // reset the ticker.
                config_tick = Instant::now();
            }

            // Trigger the camera with the software trigger as per genicam.
            camera
                .driver
                .software_trigger()
                .expect("Failed to trigger camera with Software");

            // Attempt to take off an image. Delta for image name generation
            // and sending the payload was less than a couple microseconds.
            if let Some(buffer) = camera_stream.try_pop_buffer() {
                let delta_ms = tick.elapsed().as_millis();

                // SAFETY: This function assumes the buffer is backed by a leaked box
                #[allow(unsafe_code)]
                if let Ok(dynamic_image) = unsafe { buffer.into_image() } {
                    let utc_time = Utc::now();

                    camera_stream.push_buffer(&build_buffer());
                    if delta_ms < interval_ms {
                        let sleep_ms = interval_ms - delta_ms;
                        let payload = DevicePayload {
                            uuid,
                            image: dynamic_image,
                            datetime: utc_time,
                            location_id: camera.bed_location_id,
                        };
                        image_channel.send(payload).unwrap();
                        std::thread::sleep(Duration::from_millis(sleep_ms as u64));
                    }
                } else {
                    // Have seen instances in testing where the camera stream fails, which
                    // can be due to light, network bandwidths etc.
                    // TODO: May only need to use camera_stream.stop_thread() here which is
                    //       a soft thread stop without rebuilding the buffers. The current
                    //       implementation may be overkill, however there was limited time
                    //       to test this.
                    camera_stream.stop_thread(true);
                    camera_stream.start_thread();
                    camera_stream.push_buffer(&build_buffer());
                }
            }
        }
        barrier.wait();
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test_file_path;
    use aravis::PixelFormat;
    use serial_test::serial;
    use std::{
        fs::{self, create_dir_all},
        path::PathBuf,
        str::FromStr,
        sync::mpsc,
        thread,
    };

    #[test]
    #[serial]
    fn test_write_camera_configs() {
        let ips: Vec<(u8, &str, u8)> = vec![
            (0, "169.254.8.10", 0),
            (1, "169.254.8.11", 1),
            (2, "169.254.8.12", 0),
            (3, "169.254.8.13", 1),
            (4, "169.254.8.14", 0),
            (5, "169.254.8.15", 1),
        ];

        for (id, ip, bed_id) in ips {
            let mut config =
                OnyxCameraConfig::new(Ipv4Addr::from_str(ip).expect("Failed to create address"), 3);
            config.roi = Some(Roi {
                x: 0,
                y: 0,
                h: 1024,
                w: 1280,
            });
            config.pixel_format = Some(CameraPixelFormat(PixelFormat::BAYER_RG_8));
            config.acquisition_mode = Some(WrapperAcquisitionMode(AcquisitionMode::Continuous));
            config.auto_packet_size = Some(true);
            config.trigger = Some(DeviceTrigger::Software);
            config.bed_location_id = Some(bed_id);
            config.auto_brightness = Some(true);
            config.auto_gain = Some(true);
            config.exposure_min = Some(100);
            config.exposure_max = Some(30000);
            config.auto_exposure = Some(true);

            let f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(format!(
                    "{}/config/devices/crop_bed/camera_{id}.yaml",
                    env!("CARGO_MANIFEST_DIR")
                ))
                .expect("Couldn't open file");

            serde_yaml::to_writer(f, &config).unwrap();

            let x = std::fs::File::open(format!(
                "{}/config/devices/crop_bed/camera_{id}.yaml",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("Could not open file.");

            let read_config: OnyxCameraConfig =
                serde_yaml::from_reader(&x).expect("Could not read values.");

            assert!(config == read_config, "Failed to be created equally");
        }
    }

    #[cfg_attr(not(feature = "hardware_test"), ignore)]
    #[test]
    #[serial]
    /// Test camera capture without needing to create a component. Following 
    /// this type of development is helpful when trouble shooting new device 
    /// implementations.
    fn test_camera_run_without_component() {
        let file = test_file_path!("/config/devices/crop_bed/camera_0.yaml");
        let camera = OnyxCamera::from_config_file(file);
        let config = OnyxCameraConfig::from_file(file);

        let barrier = Arc::new(Barrier::new(1));
        let stop_signal = Arc::new(AtomicBool::new(false));
        let (device_channel_tx, device_channel_rx) = mpsc::channel::<DevicePayload>();

        let controller_stop_signal = stop_signal.clone();

        // Start the devices doing the work on separate threads.
        let controller_handle = thread::spawn(|| {
            CameraController::start(camera, controller_stop_signal, barrier, device_channel_tx);
        });

        // Start a writing thread that deals with sending the images to disk.
        let component_handle = thread::spawn(|| {
            let mut write_hanldes = Vec::new();
            for payload in device_channel_rx {
                // Each time a new image is retrieved write it to disk in its own thread.
                let handle = thread::spawn(move || {
                    let mut path = PathBuf::from("./test-outputs/device-tests/camera/0");
                    path.push(payload.filename());
                    create_dir_all(path.parent().expect("Error in defining file path"))
                        .expect("Failed to create filepath");
                    if let Err(e) = payload.image.save(&path) {
                        println!("failed to save image to path {e}");
                    }
                });
                write_hanldes.push(handle);
            }
            write_hanldes
        });

        thread::sleep(Duration::from_secs(5));

        stop_signal.store(true, Ordering::Relaxed);

        controller_handle
            .join()
            .expect("Failed to safely exist the thread");
        let write_handles = component_handle
            .join()
            .expect("Failed to safely exist the thread");
        for handle in write_handles {
            handle.join().expect("Faild exit writes safely");
        }

        let images_count = fs::read_dir("./test-outputs/device-tests/camera/0/0")
            .expect("Failed to read dir")
            .count();

        let expected = 5 * config.fps;

        assert!(
            images_count.abs_diff(expected as usize) < 5,
            "Failed to generate the corrent number of images Actual {}, Expected {}",
            images_count,
            expected
        );
    }
}
