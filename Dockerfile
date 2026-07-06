# Build Stage
FROM rust:alpine AS builder

# Install build dependencies, including git, static OpenSSL, build-base, and cmake
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig git build-base cmake

WORKDIR /usr/src/little-bobby-tabots

# Force static linking of OpenSSL
ENV OPENSSL_STATIC=1
ENV OPENSSL_DIR=/usr

# Copy dependency manifests and build a dummy main to cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

# Remove dummy build artifacts and copy the actual source code
RUN rm -f target/release/deps/little_bobby_tabots* src/main.rs
COPY src ./src

# Compile the final release binary
RUN cargo build --release

# Runtime Stage
FROM alpine:latest

# Install python3 and ffmpeg
RUN apk add --no-cache ffmpeg python3 curl

# Install uv and use it to install the latest yt-dlp securely
RUN curl -LsSf https://astral.sh/uv/install.sh | sh && \
    /root/.local/bin/uv tool install yt-dlp

ENV PATH="/root/.local/bin:${PATH}"

# Copy the compiled static binary from the builder stage
COPY --from=builder /usr/src/little-bobby-tabots/target/release/little-bobby-tabots /usr/local/bin/little-bobby-tabots

# Set runtime env defaults
ENV DISCORD_TOKEN=""
ENV GUILD_ID=""
ENV RUST_LOG="info"

CMD ["little-bobby-tabots"]
