//! Native preview rendering uses a separate thread pool
//! Web requests render synchronously on the UI thread

#[cfg(any(target_arch = "wasm32", test))]
use crate::orb_shape::OrbShape;
use crate::preview_request::PreviewRequest;

/// Request key and rendered image; None on failure
pub type PreviewResult = (String, Option<egui::ColorImage>);

/// The preview worker, started by the first request
#[derive(Default)]
pub enum PreviewWorker {
    #[default]
    NotStarted,
    /// Native worker; dropping the request sender ends it after pending work
    #[cfg(not(target_arch = "wasm32"))]
    Running { requests: std::sync::mpsc::Sender<PreviewRequest>, results: std::sync::mpsc::Receiver<PreviewResult> },
    /// The web: requests render as they are made
    #[cfg(any(target_arch = "wasm32", test))]
    Inline { shape: OrbShape, results: Vec<PreviewResult> },
    /// The worker could not start or stopped; requests are refused
    // The web has no thread to stop
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Stopped,
}

impl PreviewWorker {
    /// Submit work, starting the worker on first use; false if it has stopped
    /// Completed results request a repaint so the UI polls them on the next pass
    pub fn request(&mut self, context: &egui::Context, request: PreviewRequest) -> bool {
        if matches!(self, PreviewWorker::NotStarted) {
            *self = PreviewWorker::start(context);
        }

        match self {
            #[cfg(not(target_arch = "wasm32"))]
            PreviewWorker::Running { requests, .. } => {
                if requests.send(request).is_ok() {
                    return true;
                }

                *self = PreviewWorker::Stopped;
                false
            }
            #[cfg(any(target_arch = "wasm32", test))]
            PreviewWorker::Inline { shape, results } => {
                results.push((request.key.clone(), request.render(shape)));
                context.request_repaint();
                true
            }
            PreviewWorker::NotStarted | PreviewWorker::Stopped => false,
        }
    }

    /// The results ready since the last call
    pub fn poll(&mut self) -> Vec<PreviewResult> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            PreviewWorker::Running { results, .. } => {
                use std::sync::mpsc::TryRecvError;

                let mut ready = Vec::new();
                loop {
                    match results.try_recv() {
                        Ok(result) => ready.push(result),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            log::warn!("the shelf preview worker stopped");
                            *self = PreviewWorker::Stopped;
                            break;
                        }
                    }
                }
                ready
            }
            #[cfg(any(target_arch = "wasm32", test))]
            PreviewWorker::Inline { results, .. } => std::mem::take(results),
            PreviewWorker::NotStarted | PreviewWorker::Stopped => Vec::new(),
        }
    }

    /// Whether the worker stopped: it refuses requests and answers none of the queued ones
    pub fn is_stopped(&self) -> bool {
        matches!(self, PreviewWorker::Stopped)
    }

    /// Start the worker thread (Stopped when it cannot start)
    #[cfg(not(target_arch = "wasm32"))]
    fn start(ctx: &egui::Context) -> PreviewWorker {
        use std::collections::VecDeque;
        use std::sync::mpsc::channel;

        use crate::orb_shape::OrbShape;

        let (requests, request_rx) = channel::<PreviewRequest>();
        let (result_tx, results) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("texturecraft-shelf-previews".into()).spawn(move || {
            // A separate pool keeps previews from queuing ahead of the UI's work
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1));
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("texturecraft-preview-{i}")).build();
            if let Err(e) = &pool {
                log::warn!("shelf previews share the global thread pool: {e}");
            }
            let mut shape = OrbShape::default();
            let mut queue = VecDeque::new();
            loop {
                if queue.is_empty() {
                    let Ok(request) = request_rx.recv() else { break };

                    queue.push_back(request);
                }
                queue.extend(request_rx.try_iter());

                let Some(request) = next_request(&mut queue) else { continue };

                let image = match &pool {
                    Ok(pool) => pool.install(|| request.render(&mut shape)),
                    Err(_) => request.render(&mut shape),
                };
                if result_tx.send((request.key, image)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });

        match spawned {
            Ok(_) => PreviewWorker::Running { requests, results },
            Err(e) => {
                log::warn!("could not start the shelf preview worker: {e}");
                PreviewWorker::Stopped
            }
        }
    }

    /// Render inline (no threads on the web)
    #[cfg(target_arch = "wasm32")]
    fn start(_ctx: &egui::Context) -> PreviewWorker {
        PreviewWorker::Inline { shape: OrbShape::default(), results: Vec::new() }
    }
}

/// Prioritize the newest hover preview over older requests
#[cfg(not(target_arch = "wasm32"))]
fn next_request(queue: &mut std::collections::VecDeque<PreviewRequest>) -> Option<PreviewRequest> {
    match queue.iter().rposition(|request| request.hover) {
        Some(newest_hover) => queue.remove(newest_hover),
        None => queue.pop_front(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use texturecraft_engine::materials::MATERIALS;
    use texturecraft_engine::smart_masks::SMART_MASKS;

    use super::*;
    use crate::shelf_preview::ShelfPreview;

    #[test]
    fn inline_results_schedule_a_frame_to_display_them() {
        for id in ["steel", "nope"] {
            let context = egui::Context::default();
            for _ in 0..3 {
                let mut output = context.run_pass(Default::default(), |_| {});

                // No renderer consumes the texture updates in this scheduling test
                output.textures_delta.clear();
            }
            assert!(!context.has_requested_repaint(), "the UI is idle before the request");

            let mut worker = PreviewWorker::Inline { shape: OrbShape::default(), results: Vec::new() };
            assert!(worker.poll().is_empty(), "the UI polls before submitting work");
            let request = PreviewRequest { key: id.into(), preview: ShelfPreview::Material(id), size_px: 8, hover: false };
            assert!(worker.request(&context, request));

            let results = worker.poll();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].1.is_some(), id == "steel");
            assert!(context.has_requested_repaint(), "the result needs another frame even when this is the only request");
        }
    }

    #[test]
    fn requests_are_answered_and_unknown_assets_get_none() {
        let ctx = egui::Context::default();
        let mut worker = PreviewWorker::default();
        let size_px = 32;
        let requests = [
            ("material", ShelfPreview::Material(MATERIALS[0].id)),
            ("unknown material", ShelfPreview::Material("nope")),
            ("mask", ShelfPreview::SmartMask(SMART_MASKS[0].id)),
            ("unknown mask", ShelfPreview::SmartMask("nope")),
            ("smart material", ShelfPreview::SmartMaterial("steel_painted_worn")),
            ("unknown smart material", ShelfPreview::SmartMaterial("nope")),
        ];
        let requests_sent = requests.len();
        for (key, preview) in requests {
            assert!(worker.request(&ctx, PreviewRequest { key: key.into(), preview, size_px, hover: false }), "{key}");
        }

        let mut answers = HashMap::new();
        let start = Instant::now();
        while answers.len() < requests_sent && start.elapsed() < Duration::from_secs(120) {
            answers.extend(worker.poll());
            std::thread::sleep(Duration::from_millis(10));
        }

        for key in ["material", "mask", "smart material"] {
            let image = answers.get(key).and_then(Option::as_ref).unwrap_or_else(|| panic!("{key}: {answers:?}"));
            assert_eq!(image.size, [size_px, size_px], "{key}");
        }
        for key in ["unknown material", "unknown mask", "unknown smart material"] {
            assert!(matches!(answers.get(key), Some(None)), "{key}: {answers:?}");
        }
        assert!(!worker.is_stopped());
    }

    #[test]
    fn the_newest_hover_preview_goes_first_then_the_oldest_request() {
        let request = |key: &str, hover| PreviewRequest { key: key.into(), preview: ShelfPreview::Material("steel"), size_px: 8, hover };
        let mut queue: std::collections::VecDeque<_> =
            [request("thumbnail 1", false), request("old hover", true), request("thumbnail 2", false), request("new hover", true)].into();

        let order: Vec<String> = std::iter::from_fn(|| next_request(&mut queue)).map(|r| r.key).collect();
        assert_eq!(order, ["new hover", "old hover", "thumbnail 1", "thumbnail 2"]);
    }
}
