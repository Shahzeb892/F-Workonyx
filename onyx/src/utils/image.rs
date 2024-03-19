use aravis::PixelFormat;
use serde::{de::Visitor, Deserialize, Serialize, Serializer};

/// Wrapper type for implementing serde for pixel format
/// configuration.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CameraPixelFormat(pub PixelFormat);

impl Serialize for CameraPixelFormat {
    // TODO: add in the other variants of this.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            PixelFormat::BAYER_RG_8 => {
                serializer.serialize_unit_variant("PixelFormat", 0, "BAYER_RG_8")
            }
            _ => panic!("Un configured pixel format"),
        }
    }
}

impl<'de> Deserialize<'de> for CameraPixelFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(PixelFormatVisitor {})
    }
}

/// Wrapper type for serde implementation.
pub struct PixelFormatVisitor {}

impl<'de> Visitor<'de> for PixelFormatVisitor {
    type Value = CameraPixelFormat;

    // TODO: Update function to elided lifetime below. Good first issue.
    #[allow(unused_attributes)]
    #[allow(elided_lifetimes_in_paths)]
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Could not deserialise CameraPixelFormat")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match v {
            "RGB_8_PACKER" => Ok(CameraPixelFormat(PixelFormat::RGB_8_PACKED)),
            "BAYER_RG_8" => Ok(CameraPixelFormat(PixelFormat::BAYER_RG_8)),
            "RGB_8_PLANAR" => Ok(CameraPixelFormat(PixelFormat::RGB_8_PLANAR)),
            _ => Err(serde::de::Error::custom("Unknown pixel format {v:?}")),
        }
    }
}

/// Region of interest to select from within a camera frame.
/// This is useful to tune if you need to reduce the bandwidth 
/// of the network devices and send smaller image segments.
/// Ref: p.g. 88 Genicam Standard.
#[derive(Deserialize, Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Roi {
    /// X offset from upper left of the image.
    pub x: i32,
    /// y offset from upper left of the image.
    pub y: i32,
    /// Width in x.
    pub w: i32,
    /// Height in y.
    pub h: i32,
}
