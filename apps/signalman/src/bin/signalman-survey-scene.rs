// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
use signalman::survey::{
    AnalysisSettings, CaptureLimits, ImportLimits, Survey, import_capture_json, import_gpx,
};
use std::io::{Read, Write};
fn read(path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (survey, output) = if args.first().is_some_and(|a| a == "--gpx")
        && (args.len() == 3 || args.len() == 4)
    {
        let track = import_gpx(
            &read(&args[1])?,
            ImportLimits {
                max_bytes: 4 * 1024 * 1024,
                max_track_points: 4096,
                max_xml_depth: 32,
            },
        )
        .map_err(|e| format!("GPX refused: {e:?}"))?;
        let captures = if args.len() == 4 {
            vec![
                import_capture_json(
                    &read(&args[3])?,
                    CaptureLimits {
                        max_bytes: 4 * 1024 * 1024,
                        max_records: 4096,
                        max_unknown_fields: 256,
                    },
                )
                .map_err(|e| format!("Capture refused: {e:?}"))?,
            ]
        } else {
            vec![]
        };
        // Join settings are not used by this presentation. No clock mapping,
        // event/track association or coverage estimate is performed.
        (
            Survey {
                schema_version: 1,
                tracks: vec![track],
                captures,
                settings: AnalysisSettings {
                    clock_mappings: vec![],
                    interpolation_gap_ms: 1000,
                    freshness_window_ms: 1000,
                    spatial_bucket_m: 10,
                    selected_sources: vec![],
                },
            },
            &args[2],
        )
    } else if args.len() == 2 {
        (
            signalman::survey::import_survey(&read(&args[0])?, 4 * 1024 * 1024)
                .map_err(|e| format!("Survey import refused: {e:?}"))?,
            &args[1],
        )
    } else {
        return Err("usage: signalman-survey-scene INPUT.survey.json OUTPUT.scene.json | --gpx TRACK.gpx OUTPUT.scene.json [CAPTURE.json]".into());
    };
    let recipe = signalman::survey::scene::survey_scene(&survey)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?
        .write_all(&serde_json::to_vec_pretty(&recipe)?)?;
    println!("Exported supplied survey tracks; radio positions and coverage remain unknown");
    Ok(())
}
