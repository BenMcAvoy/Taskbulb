# Taskbulb

A small Rust application for controlling a smart light through a [Home Assistant](https://www.home-assistant.io/) instance from a Windows taskbar notification-area (system tray) icon

Taskbulb is designed to control a room's smart light without pop-ups or web technologies, keeping the experience unobtrusive

## Requirements

- A running [Home Assistant](https://www.home-assistant.io/) instance with a light entity
- The [Rust toolchain](https://www.rust-lang.org/tools/install), including [Cargo](https://doc.rust-lang.org/cargo/)
- Windows for the full feature set, including the taskbar controls and global mouse-wheel shortcuts. (basic toggling is the only fully cross platform feature)

## Building

1. Clone the repository:

   ```text
   git clone https://github.com/BenMcAvoy/Taskbulb.git
   cd Taskbulb
   ```

2. Build the release executable:

   ```text
   cargo build --release -j 4
   ```

   The executable will be created at `./target/release/taskbulb.exe` on Windows. The file extension is platform-dependent

## Configuration

1. Copy `.env.example` to `.env`
2. Fill in the following values:

   | Variable | Description |
   | --- | --- |
   | `HA_BASE_URL` | The base URL of your Home Assistant instance, without a trailing slash (for example, `http://homeassistant.local:8123`) |
   | `HA_TOKEN` | A [long-lived access token](https://www.home-assistant.io/docs/authentication/) created from your Home Assistant user profile's **Security** tab in the website |
   | `HA_ENTITY_ID` | The light's entity ID, e.g. `light.bedroom`, from Home Assistant's [entities](https://www.home-assistant.io/docs/configuration/entities_domains/) list on the website |

3. Run the executable:

   ```text
   .\target\release\taskbulb.exe
   ```
