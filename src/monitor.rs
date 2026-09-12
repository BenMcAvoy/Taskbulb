use std::time::Duration;

use tokio::sync::{broadcast, mpsc};

const COALESCE_DELAY: Duration = Duration::from_millis(50);

pub struct MonitorController {
    command_tx: mpsc::UnboundedSender<i16>,
    updates: broadcast::Sender<u8>,
}

impl MonitorController {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (updates, _) = broadcast::channel(16);

        tokio::spawn(run_worker(command_rx, updates.clone()));

        Self {
            command_tx,
            updates,
        }
    }

    pub fn queue_brightness_step(&self, step: i16) {
        let _ = self.command_tx.send(step);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<u8> {
        self.updates.subscribe()
    }

    pub async fn brightness(&self) -> Option<u8> {
        tokio::task::spawn_blocking(current_brightness)
            .await
            .ok()
            .flatten()
    }
}

async fn run_worker(mut commands: mpsc::UnboundedReceiver<i16>, updates: broadcast::Sender<u8>) {
    while let Some(first_step) = commands.recv().await {
        tokio::time::sleep(COALESCE_DELAY).await;
        let step = collect_steps(first_step, &mut commands);

        if step == 0 {
            continue;
        }

        let _ = tokio::task::spawn_blocking(move || adjust_all_brightness(step)).await;
        if let Ok(Some(brightness)) = tokio::task::spawn_blocking(current_brightness).await {
            let _ = updates.send(brightness);
        }
    }
}

fn collect_steps(first_step: i16, commands: &mut mpsc::UnboundedReceiver<i16>) -> i16 {
    let mut total = first_step;
    while let Ok(step) = commands.try_recv() {
        total = total.saturating_add(step);
    }
    total
}

#[cfg(target_os = "windows")]
mod windows {
    use windows_sys::Win32::{
        Devices::Display::{
            DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
            GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR, SetMonitorBrightness,
        },
        Graphics::Gdi::{EnumDisplayMonitors, HMONITOR},
    };

    struct BrightnessTotals {
        total_percent: f64,
        monitor_count: u32,
    }

    pub(super) fn adjust_all_brightness(step: i16) {
        unsafe extern "system" fn callback(
            monitor: HMONITOR,
            _dc: windows_sys::Win32::Graphics::Gdi::HDC,
            _rect: *mut windows_sys::Win32::Foundation::RECT,
            data: windows_sys::Win32::Foundation::LPARAM,
        ) -> windows_sys::core::BOOL {
            let step = unsafe { *(data as *const i32) };
            with_physical_monitors(monitor, |physical_monitor| {
                let mut minimum = 0;
                let mut current = 0;
                let mut maximum = 0;

                if unsafe {
                    GetMonitorBrightness(
                        physical_monitor.hPhysicalMonitor,
                        &mut minimum,
                        &mut current,
                        &mut maximum,
                    )
                } != 0
                {
                    let brightness =
                        (current as i32 + step).clamp(minimum as i32, maximum as i32) as u32;
                    let _ = unsafe {
                        SetMonitorBrightness(physical_monitor.hPhysicalMonitor, brightness)
                    };
                }
            });
            1
        }

        let step = step as i32;
        unsafe {
            let _ = EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(callback),
                &step as *const i32 as isize,
            );
        }
    }

    pub(super) fn current_brightness() -> Option<u8> {
        unsafe extern "system" fn callback(
            monitor: HMONITOR,
            _dc: windows_sys::Win32::Graphics::Gdi::HDC,
            _rect: *mut windows_sys::Win32::Foundation::RECT,
            data: windows_sys::Win32::Foundation::LPARAM,
        ) -> windows_sys::core::BOOL {
            let totals = unsafe { &mut *(data as *mut BrightnessTotals) };
            with_physical_monitors(monitor, |physical_monitor| {
                let mut minimum = 0;
                let mut current = 0;
                let mut maximum = 0;

                if unsafe {
                    GetMonitorBrightness(
                        physical_monitor.hPhysicalMonitor,
                        &mut minimum,
                        &mut current,
                        &mut maximum,
                    )
                } != 0
                    && maximum > minimum
                {
                    totals.total_percent += (current.saturating_sub(minimum) as f64 * 100.0)
                        / (maximum - minimum) as f64;
                    totals.monitor_count += 1;
                }
            });
            1
        }

        let mut totals = BrightnessTotals {
            total_percent: 0.0,
            monitor_count: 0,
        };
        unsafe {
            let _ = EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(callback),
                &mut totals as *mut BrightnessTotals as isize,
            );
        }

        (totals.monitor_count > 0)
            .then(|| (totals.total_percent / totals.monitor_count as f64).round() as u8)
    }

    fn with_physical_monitors<F>(monitor: HMONITOR, mut action: F)
    where
        F: FnMut(&PHYSICAL_MONITOR),
    {
        let mut count = 0;
        if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count) } == 0
            || count == 0
        {
            return;
        }

        let mut physical_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
        if unsafe {
            GetPhysicalMonitorsFromHMONITOR(monitor, count, physical_monitors.as_mut_ptr())
        } == 0
        {
            return;
        }

        for physical_monitor in &physical_monitors {
            action(physical_monitor);
        }

        let _ = unsafe { DestroyPhysicalMonitors(count, physical_monitors.as_ptr()) };
    }
}

#[cfg(target_os = "windows")]
fn adjust_all_brightness(step: i16) {
    windows::adjust_all_brightness(step);
}

#[cfg(not(target_os = "windows"))]
fn adjust_all_brightness(_step: i16) {}

#[cfg(target_os = "windows")]
fn current_brightness() -> Option<u8> {
    windows::current_brightness()
}

#[cfg(not(target_os = "windows"))]
fn current_brightness() -> Option<u8> {
    None
}
