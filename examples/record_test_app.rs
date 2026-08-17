//! A deliberately flawed native UI for manually exercising `framewatch record`.
//!
//! The visible mistakes are intentional. See `docs/RECORD_TEST_APP.md` for the
//! runbook and answer key.

use eframe::egui::{self, Color32, RichText, Stroke};
use std::time::{Duration, Instant};

struct RecordingTestApp {
    started: Instant,
    submit_clicks: u32,
    notifications: bool,
    email: String,
}

impl Default for RecordingTestApp {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            submit_clicks: 0,
            notifications: true,
            // PLANTED 5: the default contact value is not a valid email address.
            email: "not-an-email".into(),
        }
    }
}

impl eframe::App for RecordingTestApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(100));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                // PLANTED 1: "Recording" is misspelled.
                ui.heading("Recordng Quality Check");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Dashboard v2.5.0").strong());
                });
            });
            ui.label("A deliberately flawed screen for narrated Framewatch testing.");
            ui.add_space(10.0);

            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 33, 43))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!(
                            "Live test clock: {:.1}s",
                            self.started.elapsed().as_secs_f32()
                        ));
                    });
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        // PLANTED 2: the two status messages contradict each other.
                        ui.colored_label(Color32::LIGHT_GREEN, "● All systems operational");
                        ui.colored_label(Color32::LIGHT_RED, "3 critical errors");
                    });
                });

            ui.add_space(10.0);
            ui.columns(2, |columns| {
                columns[0].group(|ui| {
                    ui.heading("Deployment progress");
                    // PLANTED 3: the bar is 72%, while its label says 42%.
                    ui.add(egui::ProgressBar::new(0.72).text("42% complete"));
                    ui.add_space(8.0);
                    ui.label("Estimated charges");
                    // PLANTED 4: deliberately incorrect arithmetic.
                    ui.monospace("$10.00 + $5.00 = $12.00");
                });

                columns[1].group(|ui| {
                    ui.heading("Account settings");
                    ui.label("Contact email");
                    ui.text_edit_singleline(&mut self.email);
                    // PLANTED 6: spelling mistake in a user-facing label.
                    ui.checkbox(&mut self.notifications, "Enable notificatons");
                    // PLANTED 7: intentionally poor contrast on the dark panel.
                    ui.colored_label(
                        Color32::from_gray(70),
                        "We will send important security alerts here.",
                    );
                });
            });

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                // PLANTED 8: destructive and safe action colors are reversed.
                let delete = egui::Button::new("Delete workspace")
                    .fill(Color32::from_rgb(30, 150, 75))
                    .stroke(Stroke::new(1.0, Color32::LIGHT_GREEN));
                let save = egui::Button::new("Save changes")
                    .fill(Color32::from_rgb(175, 40, 45))
                    .stroke(Stroke::new(1.0, Color32::LIGHT_RED));
                ui.add(delete);
                ui.add(save);

                // PLANTED 9: typo in the primary action.
                if ui.button("SUBMITT").clicked() {
                    self.submit_clicks += 1;
                }
                ui.label(format!("Submitted {} time(s)", self.submit_clicks));
            });

            ui.add_space(16.0);
            ui.separator();
            // PLANTED 10: footer version conflicts with the header version.
            ui.label(RichText::new("Framewatch demo • version 2.4.0").small());
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // Intentionally odd so the first recording frame exercises the
            // H.264 dimension-padding fix.
            .with_inner_size([901.0, 651.0])
            .with_min_inner_size([700.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Framewatch Recording Test",
        options,
        Box::new(|_cc| Ok(Box::new(RecordingTestApp::default()))),
    )
}
