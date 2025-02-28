# Use a Rust base image with Cargo installed
FROM rust:1.82.0 AS builder

# Install environment dependency
RUN apt-get update && apt-get install -y libssl-dev

# Set the working directory inside the container
WORKDIR /usr/src/app

COPY ./ ./
COPY ./.env ./.env

# Build your application
RUN cargo build --release

# Start a new stage to create a smaller image without unnecessary build dependencies
FROM debian:bookworm-slim AS runtime

# Install OpenSSL - it is dynamically linked by some of our dependencies 
# Install ca-certificates - it is needed to verify TLS certificates
# when establishing HTTPS connections
RUN apt-get update -y \
    && apt-get install -y --no-install-recommends libssl-dev openssl ca-certificates \
# Clean up
    && apt-get autoremove -y \
    && apt-get clean -y \
    && rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/src/app

# Copy the built binary from the previous stage
COPY --from=builder /usr/src/app/target/release/cron-bot cron-bot
COPY --from=builder /usr/src/app/.env .env

# Command to run the application
CMD ["./cron-bot"]