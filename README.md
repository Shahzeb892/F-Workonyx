# ONYX

The onyx control system uses several open source patterns used in micro-service design, as highlighted in
virtual forums for improvements to the [software development life-cycle of vehicles](https://www.redhat.com/en/resources/sdv-open-source-accelerates-innovation-whitepaper). 
Using this common pattern,functionality is separated out, making iterations and code management easier
as the project progresses; rather than managing a singular and highly coupled monolithic binary.

In the first iteration of the minimum viable product, the onyx workspace is separated into two areas,
the library of devices and components, and the systems that use these components as individual binaries.
This means if a new system is required by a customer, the required devices can be bundled into a component
which then performs the objective function of the new system.

A device is an atomic unit that commands can be applied to to achieve many different things, the concrete
example of this is the Camera device. The Camera device provides access to an underlying driver written
and maintained by another group (the [aravis](https://github.com/AravisProject/aravis) project), if no such driver exists, then you must create one
from a specification (like the IX3212 power modules). The onyx device provides a thin wrapper around more
expressive functionality that the full driver provides, making developers lives much easier when structuring
them together with other onyx devices to make a component.

Unlike a device, a component is a collection of atomic units that performs some function which is repeated
across different parts of the machine. The concrete example of this is a Crop Bed Camera Array, which manages
the cameras that are housed within one crop bed module. Using this as a component means that each crop bed
can operate independently from another, allowing for greater flexibility. A component must control how devices
are used in concert, and may spawn tasks in multiple threads. This means a component must also keep track of
the threads it has created, and gracefully shut them down in conclusion.

Finally components can be grouped together to make a system, these systems are not libraries, but binaries
that are created by using the components from the onyx library. These binaries can then be bundled with
other dynamic dependencies in lightweight containers. Currently these systems are very simple, however 
extending them should be extremely trivial. You will need to amend the git work flow to ensure any new
binaries are recompiled and distributed to the container registry.

--- 

## High level TODO:

- [ ] Resolve error handling strategy.
- [ ] Review the clippy allow macros.
- [ ] Implement some form of logging and telemetry.
- [ ] Update main CICD for cargo clippy check & fail.
- [ ] Restructuring the GitHub repo to a library, and separate repo binary crates.

> [!NOTE]  
> For a comprehensive list of todos, grep the codebase for TODO:

--- 

> **Warning**
> Licenses and libraries used in this control system have not been validated.

---

The onyx control system is built with two concepts, the library of devices and components and the system binaries.

## Library Structure.

``` graphql
├── Cargo.toml
├── config
│   ├── components
│   │   └── crop_bed
│   │       ├── actuating
│   │       │   ├── lighting
│   │       │   │   └── crop_bed_lighting.yaml
│   │       │   └── power
│   │       │       ├── crop_bed_power_0_no_map.yaml
│   │       │       ├── crop_bed_power_0.yaml
│   │       │       ├── crop_bed_power_1_no_map.yaml
│   │       │       ├── crop_bed_power_1.yaml
│   │       │       ├── crop_bed_power_2_no_map.yaml
│   │       │       └── crop_bed_power_2.yaml
│   │       └── sensing
│   │           └── camera_array
│   │               ├── crop_bed_array_test.yaml
│   │               ├── crop_bed_camera_array_0.yaml
│   │               ├── crop_bed_camera_array_1.yaml
│   │               └── crop_bed_camera_array_2.yaml
│   └── devices
│       └── crop_bed
│           ├── camera_0.yaml
│           ├── camera_1.yaml
│           ├── camera_2.yaml
│           ├── camera_3.yaml
│           ├── camera_4.yaml
│           ├── camera_5.yaml
│           ├── pdm_0.yaml
│           ├── pdm_1.yaml
│           └── pdm_utilities.yaml
├── src
│   ├── components
│   │   └── crop_bed
│   │       ├── actuating
│   │       │   ├── lighting.rs
│   │       │   └── power.rs
│   │       └── sensing
│   │           └── camera_array.rs
│   ├── components.rs
│   ├── devices
│   │   ├── hardware
│   │   │   ├── camera.rs
│   │   │   └── pdm.rs
│   │   └── software
│   ├── devices.rs
│   ├── lib.rs
│   ├── messages
│   │   ├── control
│   │   │   ├── light.rs
│   │   │   └── weed.rs
│   │   └── logging
│   ├── messages.rs
│   ├── utils
│   │   ├── image.rs
│   │   └── tests.rs
│   └── utils.rs
└── test-outputs
    └── component-tests
        └── camera_array
            └── 0
                └── 0

```

## System Structure.

``` graphql
.
├── crop_bed
│   ├── image_capture
│   │   ├── Cargo.toml
│   │   └── src
│   │       └── main.rs
│   ├── lighting
│   │   ├── Cargo.toml
│   │   └── src
│   │       └── main.rs
│   └── spray
│       ├── Cargo.toml
│       └── src
│           └── main.rs
└── utilities
    └── speed_measurement
        ├── Cargo.toml
        └── src
            └── main.rs


```


