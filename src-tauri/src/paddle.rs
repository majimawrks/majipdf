//! Accurate OCR engine: PaddleOCR PP-OCRv5 (mobile det + latin rec) on ONNX Runtime, loaded from the bundled DLL.
//! Ported from `_spike/ocr_engines` (`paddle_batch`, batch 1: padding made batching slower). See _docs/paddle-contract.md.
use crate::ocrlayer::Word;
use crate::runtime;
use image::RgbImage;
use ort::session::{builder::GraphOptimizationLevel, builder::SessionBuilder, Session};
use paddle_ocr_rs::{base_net::BaseNet, db_net::DbNet, ocr_utils::OcrUtils, scale_param::ScaleParam};

const PADDING: u32 = 50;
/// Detection size cap: ~4x faster than full 300 DPI, spike F1 94.7 vs 95.0 on original scans (user 2026-10-21).
const DET_MAX_SIDE: u32 = 1280;

pub struct Paddle {
    det: DbNet,
    rec: Session,
    input: String,
    keys: Vec<String>,
}

fn l3(b: SessionBuilder) -> Result<SessionBuilder, ort::Error> {
    b.with_optimization_level(GraphOptimizationLevel::Level3)
}

impl Paddle {
    /// Err on any missing file, DLL or model problem: the caller falls back to Fast.
    pub fn load() -> Result<Paddle, String> {
        let dir = runtime::paddle_dir().ok_or("accurate OCR files are unavailable")?;
        let (dll, det, rec, dict) = (dir.join(runtime::PADDLE_ORT), dir.join(runtime::PADDLE_DET), dir.join(runtime::PADDLE_REC), dir.join(runtime::PADDLE_DICT));
        for f in [&dll, &det, &rec, &dict] {
            if !f.is_file() {
                return Err(format!("{} is missing", f.display()));
            }
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        // ponytail: ort's dynamic loader panics when the DLL can't be loaded; that becomes a plain Err here.
        std::panic::catch_unwind(|| ort::init_from(dll.display().to_string()).commit().map(|_| ()).map_err(|e| e.to_string()))
            .map_err(|_| "ONNX Runtime could not be loaded".to_string())??;
        let mut db = DbNet::new();
        BaseNet::init_model(&mut db, &det.display().to_string(), threads, Some(l3)).map_err(|e| e.to_string())?;
        let rec = Session::builder()
            .and_then(|b| b.with_optimization_level(GraphOptimizationLevel::Level3))
            .and_then(|b| b.with_intra_threads(threads))
            .and_then(|b| b.commit_from_file(&rec))
            .map_err(|e| e.to_string())?;
        let input = rec.inputs[0].name.clone();
        // the crate's dict workaround: split on '\n', index 0 = blank, last = ' '
        let keys = std::fs::read_to_string(&dict).map_err(|e| e.to_string())?.split('\n').map(String::from).collect();
        Ok(Paddle { det: db, rec, input, keys })
    }

    /// Detects and reads the text lines of one page image. `cancel` is checked between detection and recognition.
    pub fn read(&mut self, img: &RgbImage, cancel: &dyn Fn() -> Result<(), String>) -> Result<Vec<Word>, String> {
        let e = |e: &dyn std::fmt::Display| e.to_string();
        // detection (mirrors OcrLite::detect_base, padding 50) at most DET_MAX_SIDE px; recognition still reads full-res crops
        let resize = img.width().max(img.height()).min(DET_MAX_SIDE) + 2 * PADDING;
        let pimg = OcrUtils::make_padding(img, PADDING).map_err(|x| e(&x))?;
        let scale = ScaleParam::get_scale_param(&pimg, resize);
        let boxes = self.det.get_text_boxes(&pimg, &scale, 0.5, 0.3, 1.6).map_err(|x| e(&x))?;
        cancel()?;
        let crops = OcrUtils::get_part_images(&pimg, &boxes);
        let mut words = Vec::with_capacity(boxes.len());
        for (b, c) in boxes.iter().zip(&crops) {
            cancel()?; // ponytail: per line, so Cancel answers within a fraction of a second
            let w = ((c.width() as f32 * 48.0 / c.height().max(1) as f32) as u32).max(1);
            let im = image::imageops::resize(c, w, 48, image::imageops::FilterType::Triangle);
            let text = self.recognize(&im, w as usize)?;
            let pad = PADDING as f32;
            let xs = b.points.iter().map(|p| p.x as f32 - pad);
            let ys = b.points.iter().map(|p| p.y as f32 - pad);
            let (x0, x1) = xs.fold((f32::MAX, f32::MIN), |(a, z), v| (a.min(v), z.max(v)));
            let (y0, y1) = ys.fold((f32::MAX, f32::MIN), |(a, z), v| (a.min(v), z.max(v)));
            if !text.trim().is_empty() {
                words.push(Word { x0: x0.max(0.0), y0: y0.max(0.0), x1, y1, text });
            }
        }
        Ok(words)
    }

    fn recognize(&mut self, im: &RgbImage, w: usize) -> Result<String, String> {
        let mut t = vec![0f32; 3 * 48 * w];
        for (x, y, px) in im.enumerate_pixels() {
            for ch in 0..3 {
                t[(ch * 48 + y as usize) * w + x as usize] = px[ch] as f32 / 127.5 - 1.0;
            }
        }
        let tensor = ort::value::Tensor::from_array(([1usize, 3, 48, w], t)).map_err(|e| e.to_string())?;
        let outs = self.rec.run(ort::inputs![self.input.clone() => tensor]).map_err(|e| e.to_string())?;
        let (shape, data) = outs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let (steps, classes) = (shape[1] as usize, shape[2] as usize);
        let mut text = String::new();
        let mut last = 0usize;
        for s in 0..steps {
            let row = &data[s * classes..(s + 1) * classes];
            let mut mi = 0;
            for (k, v) in row.iter().enumerate() {
                if *v > row[mi] {
                    mi = k;
                }
            }
            if mi > 0 && mi < self.keys.len() && !(s > 0 && mi == last) {
                text.push_str(&self.keys[mi]);
            }
            last = mi;
        }
        Ok(text)
    }
}
