ARG BUILD_IMAGE
ARG RELEASE_IMAGE

FROM $BUILD_IMAGE as build_image

ARG BINARY
ARG RELEASE_IMAGE

ENV PATH="/home/fluxrobotics/.cargo/bin:${PATH}"

USER fluxrobotics

COPY . ./home/fluxrobotics/

WORKDIR /home/fluxrobotics/

RUN cargo build -p $BINARY --release

FROM $RELEASE_IMAGE as release_image

ARG BINARY

WORKDIR /home/fluxrobotics/
COPY --from=build_image /home/fluxrobotics/target/release/$BINARY /onyx/

ENV PATH="${PATH}:/onyx"

