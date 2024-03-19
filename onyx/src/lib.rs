/**
The onyx control system uses several open source patterns used in micro-service design,
as highlighted in virtual forums for improvements to the [software development life-cycle of vehicles]
(https://www.redhat.com/en/resources/sdv-open-source-accelerates-innovation-whitepaper).
Using this common pattern, functionality is separated out, making iterations and code management 
easier as the project progresses; rather than managing a singular and highly coupled monolithic binary.
*/

/// Components in the system are created by grouping together
/// devices into a logical unit that performs some function
/// for the overall control system.
pub mod components;
/// Devices that are an atomic unit, and can be composed 
/// with other devices into components to perform some function.
pub mod devices;
/// Message structure for communication into and out of the 
/// control system, such as process communication for the 
/// AI system.
pub mod messages;
/// Development utilities for working with serialisation and 
/// image information.
pub mod utils;
