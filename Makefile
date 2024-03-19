.PHONY: run_software_tests
run_software_tests:
	@cargo t --release --no-default-features

.PHONY: run_hardware_tests
run_hardware_tests:
	@cargo t --release --features hardware_test

.PHONY: run_camera_hardware_tests
run_camera_hardware_tests:
	@cargo t camera --release --features hardware_test

.PHONY: local_container
local_container:
	@podman build --file ./deployment/Dockerfile --build-arg BINARY=image_capture --build-arg BUILD_IMAGE=fluxrobotics/development_onyx:main --build-arg RELEASE_IMAGE=fluxrobotics/production_aravis:main --tag fluxrobotics/onyx_image_capture:local_test .
