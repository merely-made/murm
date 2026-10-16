// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! Portable survey presentation. GPX samples are landmarks, never assigned to
//! radios by matching a name or host receipt time. Captured radio identities
//! remain unplaced until an owner supplies an actual position association.
use super::{Survey, export_survey, import_survey};
use sceno::{Size2, SourceRef, Vec2};
use scenomise::backdrops::{
    BACKDROP_SCENE_VERSION, RadioDevice, SceneFacts, SceneInput, TerrainCoverage,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub fn survey_scene(survey: &Survey) -> Result<SceneInput, String> {
    let bytes = export_survey(survey).map_err(|e| format!("Survey refused: {e:?}"))?;
    let survey = import_survey(&bytes, 4 * 1024 * 1024)
        .map_err(|e| format!("Survey evidence refused: {e:?}"))?;
    let selected = &survey.settings.selected_sources;
    let admitted = |hash: &super::SourceHash| selected.is_empty() || selected.contains(hash);
    if selected.iter().any(|hash| {
        !survey.tracks.iter().any(|t| &t.source_hash == hash)
            && !survey.captures.iter().any(|c| &c.source_hash == hash)
    }) {
        return Err("A selected survey source is absent".into());
    }
    let tracks = survey
        .tracks
        .iter()
        .filter(|t| admitted(&t.source_hash))
        .collect::<Vec<_>>();
    let points = tracks
        .iter()
        .flat_map(|t| t.points.iter())
        .collect::<Vec<_>>();
    if points.is_empty() {
        return Err("Import a positioned GPX track before exporting a geographic plane".into());
    }
    let min_lat = points
        .iter()
        .map(|p| p.latitude)
        .fold(f64::INFINITY, f64::min);
    let max_lat = points
        .iter()
        .map(|p| p.latitude)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_lon = points
        .iter()
        .map(|p| p.longitude)
        .fold(f64::INFINITY, f64::min);
    let max_lon = points
        .iter()
        .map(|p| p.longitude)
        .fold(f64::NEG_INFINITY, f64::max);
    let reference_lat = (min_lat + max_lat) * 0.5;
    let metres_lat = 111_320.0;
    let metres_lon = metres_lat * reference_lat.to_radians().cos();
    if max_lon - min_lon > 180.0 || metres_lon < 1000.0 {
        return Err("Survey needs a supported local coordinate plane; polar and antimeridian spans are not inferred".into());
    }
    let extent = Size2::new(
        ((max_lon - min_lon) * metres_lon + 20.0) as f32,
        ((max_lat - min_lat) * metres_lat + 20.0) as f32,
    );
    let project = |p: &super::TrackPoint| {
        Vec2::new(
            ((p.longitude - min_lon) * metres_lon + 10.0) as f32,
            ((max_lat - p.latitude) * metres_lat + 10.0) as f32,
        )
    };
    let mut contours = Vec::new();
    let mut devices = Vec::new();
    for track in tracks {
        let digest = hex::encode(track.source_hash.0);
        let mut segments = BTreeMap::<usize, Vec<(usize, &super::TrackPoint)>>::new();
        for (i, p) in track.points.iter().enumerate() {
            segments.entry(p.track_segment).or_default().push((i, p));
        }
        for (segment, samples) in segments {
            if samples.len() > 1 {
                contours.push(samples.iter().map(|(_, p)| project(p)).collect());
            }
            let endpoints = [samples.first(), samples.last()];
            let mut previous = None;
            for (index, point) in endpoints.into_iter().flatten() {
                if previous == Some(*index) {
                    continue;
                }
                previous = Some(*index);
                let key = format!("track-{digest}/point-{index}");
                devices.push(RadioDevice {
                    key: key.clone(),
                    source: SourceRef {
                        adapter: "signalman.gpx-sample".into(),
                        id: key,
                    },
                    label: format!("Track {segment}.{index}"),
                    realm: "Recorded GPX sample; not a radio position".into(),
                    position: Some(project(point)),
                });
            }
        }
    }
    let identities = survey
        .captures
        .iter()
        .filter(|c| admitted(&c.source_hash))
        .flat_map(|c| c.records.iter())
        .flat_map(|r| std::iter::once(r.observer.as_str()).chain(r.peer.as_deref()))
        .collect::<BTreeSet<_>>();
    for id in identities {
        devices.push(RadioDevice {
            key: format!("radio-{id}"),
            source: SourceRef {
                adapter: "signalman.capture-observer".into(),
                id: id.into(),
            },
            label: id.into(),
            realm: "Capture identity; no admitted track association".into(),
            position: None,
        });
    }
    let revision = hex::encode(Sha256::digest(&bytes));
    let recipe = SceneInput {
        version: BACKDROP_SCENE_VERSION,
        source: SourceRef {
            adapter: "signalman.survey".into(),
            id: revision.clone(),
        },
        revision,
        title: format!(
            "Survey tracks · local equirectangular metres · NW {max_lat:.6},{min_lon:.6} +10m · reference latitude {reference_lat:.6} · coverage unmeasured"
        ),
        facts: SceneFacts::TerrainCoverage(TerrainCoverage {
            extent,
            contours,
            devices,
            coverage: vec![],
            unmeasured: vec![vec![
                Vec2::ZERO,
                Vec2::new(extent.w, 0.0),
                Vec2::new(extent.w, extent.h),
                Vec2::new(0.0, extent.h),
            ]],
            links: vec![],
        }),
    };
    recipe.compile().map_err(|e| e.to_string())?;
    Ok(recipe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::survey::*;
    fn survey() -> Survey {
        let track=import_gpx(br#"<gpx><trk><trkseg><trkpt lat="40.0" lon="-74.0"/><trkpt lat="40.001" lon="-73.999"/></trkseg><trkseg><trkpt lat="40.003" lon="-73.998"/><trkpt lat="40.004" lon="-73.997"/></trkseg></trk></gpx>"#,ImportLimits {max_bytes:4096,max_track_points:20,max_xml_depth:16}).unwrap();
        let capture=import_capture_json(br#"{"format":"signalman-survey-capture","schema_version":1,"records":[{"observer":"workshop","peer":"ridge","direction":"received","host_received_unix_ms":1}]}"#,CaptureLimits {max_bytes:4096,max_records:20,max_unknown_fields:20}).unwrap();
        Survey {
            schema_version: 1,
            tracks: vec![track],
            captures: vec![capture],
            settings: AnalysisSettings {
                clock_mappings: vec![],
                interpolation_gap_ms: 1000,
                freshness_window_ms: 1000,
                spatial_bucket_m: 10,
                selected_sources: vec![],
            },
        }
    }
    #[test]
    fn tracks_keep_segment_gaps_and_radios_remain_unplaced() {
        let survey = survey();
        let before = survey.clone();
        let recipe = survey_scene(&survey).unwrap();
        assert_eq!(survey, before);
        let SceneFacts::TerrainCoverage(map) = &recipe.facts else {
            unreachable!()
        };
        assert_eq!(map.contours.len(), 2);
        assert!(map.coverage.is_empty());
        assert!(
            map.devices
                .iter()
                .filter(|d| d.source.adapter == "signalman.capture-observer")
                .all(|d| d.position.is_none())
        );
        assert_eq!(recipe.compile().unwrap().unplaced.len(), 2);
        assert_eq!(
            recipe,
            SceneInput::parse(&serde_json::to_string(&recipe).unwrap()).unwrap()
        );
    }
    #[test]
    fn tampered_track_evidence_and_absent_sources_are_refused() {
        let mut source = survey();
        source.tracks[0].points[0].latitude = 41.0;
        assert!(survey_scene(&source).is_err());
        let mut source = survey();
        source.settings.selected_sources = vec![SourceHash([42; 32])];
        assert!(survey_scene(&source).is_err());
    }
}
