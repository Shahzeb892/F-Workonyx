/// Standardise how messages are sent into and out of 
/// the current control system. Provide test suite to 
/// ensure interfaces are respected.
pub mod control {
    /// Weed messages come from the AI system. They 
    /// specify weed location and timing characteristics 
    /// for when a PDM should fire.
    pub mod weed;
    /// Light messages come from another control loop. 
    /// TODO: Decide if this needs to be synchronised 
    /// with the camera software trigger.
    pub mod light;
}

/// TODO: Schedule impacted ability to implement logging.
pub mod logging {}
