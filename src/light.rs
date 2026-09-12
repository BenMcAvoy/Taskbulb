use reqwest::{Client, RequestBuilder};
use serde_json::{Value, json};
use std::env;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

#[derive(Clone)]
pub struct LightController {
    client: Client,
    base_url: String,
    token: String,
    entity_id: String,
    command_tx: mpsc::UnboundedSender<LightCommand>,
    updates: broadcast::Sender<LightState>,
}

#[derive(Clone)]
pub struct LightState {
    pub is_on: bool,
    pub brightness: i64,
    pub hue: f64,
    pub saturation: f64,
}

enum LightCommand {
    Brightness(i16),
    Hue(f64),
    Saturation(f64),
    ResetColor,
}

impl LightController {
    pub fn new(client: Client) -> Self {
        dotenvy::dotenv().ok();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (updates, _) = broadcast::channel(16);
        let controller = Self {
            client,
            base_url: env::var("HA_BASE_URL").expect("HA_BASE_URL must be set in .env"),
            token: env::var("HA_TOKEN").expect("HA_TOKEN must be set in .env"),
            entity_id: env::var("HA_ENTITY_ID").expect("HA_ENTITY_ID must be set in .env"),
            command_tx,
            updates,
        };

        let worker = controller.clone();
        tokio::spawn(async move {
            worker.command_worker(command_rx).await;
        });

        controller
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LightState> {
        self.updates.subscribe()
    }

    pub fn queue_brightness_step(&self, step: i16) {
        let _ = self.command_tx.send(LightCommand::Brightness(step));
    }

    pub fn queue_hue_step(&self, step: f64) {
        let _ = self.command_tx.send(LightCommand::Hue(step));
    }

    pub fn queue_saturation_step(&self, step: f64) {
        let _ = self.command_tx.send(LightCommand::Saturation(step));
    }

    pub fn queue_color_reset(&self) {
        let _ = self.command_tx.send(LightCommand::ResetColor);
    }

    async fn command_worker(&self, mut rx: mpsc::UnboundedReceiver<LightCommand>) {
        // Home Assistant may report hue as zero while saturation is zero. Keep the last
        // requested hue locally so hue adjustments are not lost in that state.
        let mut remembered_hue = None;

        while let Some(command) = rx.recv().await {
            // Coalesce a burst of wheel messages into one queued operation.
            tokio::time::sleep(Duration::from_millis(50)).await;

            let mut brightness_step = 0i16;
            let mut hue_step = 0.0;
            let mut saturation_step = 0.0;
            let mut reset_color = false;
            let mut add_command = |command| match command {
                LightCommand::Brightness(step) => {
                    brightness_step = brightness_step.saturating_add(step)
                }
                LightCommand::Hue(step) => hue_step += step,
                LightCommand::Saturation(step) => saturation_step += step,
                LightCommand::ResetColor => reset_color = true,
            };

            add_command(command);
            while let Ok(next) = rx.try_recv() {
                add_command(next);
            }

            let Ok(state) = self.state().await else {
                continue;
            };

            let hue = if reset_color {
                remembered_hue = Some(0.0);
                0.0
            } else if hue_step != 0.0 {
                let next_hue = (remembered_hue.unwrap_or(state.hue) + hue_step).clamp(0.0, 255.0);
                remembered_hue = Some(next_hue);
                next_hue
            } else {
                remembered_hue.unwrap_or(state.hue)
            };

            let result = if state.brightness < 30 && brightness_step < 0 {
                self.turn_off().await
            } else if !reset_color
                && brightness_step == 0
                && hue_step == 0.0
                && saturation_step == 0.0
            {
                Ok(())
            } else {
                self.apply_changes(
                    brightness_step,
                    hue,
                    if reset_color {
                        0.0
                    } else {
                        (state.saturation + saturation_step).clamp(0.0, 100.0)
                    },
                    reset_color || hue_step != 0.0 || saturation_step != 0.0,
                )
                .await
            };

            if result.is_err() {
                continue;
            }

            if let Ok(mut state) = self.state().await {
                // Hue has no observable meaning at zero saturation, so Home Assistant may
                // normalize it back to zero. Keep showing the user's selected hue locally.
                if state.saturation == 0.0
                    && let Some(hue) = remembered_hue
                {
                    state.hue = hue;
                }
                let _ = self.updates.send(state);
            }
        }
    }

    pub async fn state(&self) -> Result<LightState, reqwest::Error> {
        let response = self
            .client
            .get(format!("{}/api/states/{}", self.base_url, self.entity_id))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?;

        let value: Value = response.json().await?;
        Ok(LightState {
            is_on: value.get("state").and_then(Value::as_str) == Some("on"),
            brightness: value["attributes"]["brightness"].as_i64().unwrap_or(0),
            hue: value["attributes"]["hs_color"][0]
                .as_f64()
                .unwrap_or(0.0)
                .clamp(0.0, 255.0),
            saturation: value["attributes"]["hs_color"][1].as_f64().unwrap_or(0.0),
        })
    }

    async fn apply_changes(
        &self,
        brightness_step: i16,
        hue: f64,
        saturation: f64,
        include_color: bool,
    ) -> Result<(), reqwest::Error> {
        let mut data = json!({});
        if brightness_step != 0 {
            data["brightness_step_pct"] = json!(brightness_step);
        }
        if include_color {
            data["hs_color"] = json!([hue, saturation]);
        }
        self.call_service("turn_on", data).await
    }

    pub async fn turn_off(&self) -> Result<(), reqwest::Error> {
        self.call_service("turn_off", json!({})).await
    }

    pub async fn toggle(&self) -> Result<(), reqwest::Error> {
        self.call_service("toggle", json!({})).await
    }

    async fn call_service(&self, service: &str, mut data: Value) -> Result<(), reqwest::Error> {
        data["entity_id"] = json!(self.entity_id);
        self.service_request(service, data)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    fn service_request(&self, service: &str, data: Value) -> RequestBuilder {
        self.client
            .post(format!("{}/api/services/light/{service}", self.base_url))
            .bearer_auth(&self.token)
            .json(&data)
    }
}
