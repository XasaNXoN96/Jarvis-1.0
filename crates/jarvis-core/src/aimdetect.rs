//! The detector part of the aim assist: what an ONNX model of people looks like, how to feed it a picture and how to read its answer.
//!
//! Models of the YOLO family do NOT share one output format. Three are understood, and which one a model has is taken from the shape of
//! its output (and checked on a real trial run when the model is installed):
//!   * `ChannelsFirst` — YOLOv8 / YOLO11: output `1 × (4+classes) × anchors`: centre x, centre y, width, height, then the class scores;
//!     input RGB 0..1.
//!   * `AnchorsFirstRaw` — YOLOX as it is exported by default: `1 × anchors × (5+classes)`: x, y, w, h as offsets of the cell and the log of the
//!     size (the real box is `(x + cell) × stride`, `exp(w) × stride`), then the objectness and the class scores; the anchors are the cells of
//!     the strides 8, 16 and 32; input RGB 0..255 (no normalisation).
//!   * `AnchorsFirstDecoded` — YOLOv5 style: `1 × anchors × (5+classes)` with the boxes already in pixels; input RGB 0..1.
//! A model whose output is none of these (a classifier, a segmentation model with several outputs of another shape) is refused with the
//! reason; it is never used «at a venture».
//!
//! The model only says «a person is here» (class `person`): a box of the whole person. The «head» and the «body» that the aim assist leads
//! the crosshair to are points chosen in that box (a part of the height from the top). Exact parts of the body need a pose / keypoint model
//! or a model trained for those classes: this module does not pretend otherwise.

use crate::aimassist::{decode, Det};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    ChannelsFirst,
    AnchorsFirstRaw,
    AnchorsFirstDecoded,
}

impl Layout {
    pub fn name(self) -> &'static str {
        match self {
            Layout::ChannelsFirst => "YOLOv8 / YOLO11 (каналы × боксы)",
            Layout::AnchorsFirstRaw => "YOLOX (необработанный выход)",
            Layout::AnchorsFirstDecoded => "YOLOv5 (готовые боксы)",
        }
    }
    // the colour numbers the model expects: 0..1 (YOLOv8, YOLOv5) or 0..255 (YOLOX)
    pub fn norm(self) -> Norm {
        match self {
            Layout::AnchorsFirstRaw => Norm::Raw255,
            _ => Norm::Unit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Norm {
    Unit,
    Raw255,
}

// the answer of the model: the numbers and the shape (1 × a × b)
#[derive(Clone, Debug, PartialEq)]
pub struct RawOut {
    pub data: Vec<f32>,
    pub dims: Vec<usize>,
}

// what is known about a model that fits
#[derive(Clone, Debug, PartialEq)]
pub struct ModelSpec {
    pub layout: Layout,
    pub input_w: usize,
    pub input_h: usize,
    pub classes: usize,
    pub anchors: usize,
}

pub const STRIDES: [usize; 3] = [8, 16, 32];

// the cells of a YOLOX input: the strides 8, 16 and 32 (8400 for 640×640)
pub fn yolox_anchor_count(w: usize, h: usize) -> usize {
    STRIDES.iter().map(|s| (w / s) * (h / s)).sum()
}

// the shape of the output -> the layout, the number of classes and of anchors (or why the model is not a detector)
pub fn classify_output(dims: &[usize], input_w: usize, input_h: usize) -> Result<(Layout, usize, usize), String> {
    let bad = || format!("форма выхода модели {:?} не похожа на детектор: нужна 1×(4+классы)×боксы (YOLOv8) или 1×боксы×(5+классы) (YOLOX, YOLOv5)", dims);
    if dims.len() != 3 || dims[0] != 1 || dims[1] == 0 || dims[2] == 0 {
        return Err(bad());
    }
    let (a, b) = (dims[1], dims[2]);
    if a < b {
        if a < 5 {
            return Err(bad());
        }
        return Ok((Layout::ChannelsFirst, a - 4, b));
    }
    if b < 6 {
        // fewer than six values in the last place cannot be «5+classes»: only a short YOLOv8 answer (a few boxes) is left
        if a >= 5 && b >= 1 && a <= 8 + 4 {
            return Ok((Layout::ChannelsFirst, a - 4, b));
        }
        return Err(bad());
    }
    let classes = b - 5;
    if input_w > 0 && input_h > 0 && a == yolox_anchor_count(input_w, input_h) {
        Ok((Layout::AnchorsFirstRaw, classes, a))
    } else {
        Ok((Layout::AnchorsFirstDecoded, classes, a))
    }
}

// the boxes of the people in the pixels of the INPUT of the model; None when the numbers do not fit the shape
pub fn decode_output(out: &RawOut, spec: &ModelSpec, class: usize, min_score: f32) -> Option<Vec<Det>> {
    if out.dims.len() != 3 || out.dims[0] != 1 || class >= spec.classes {
        return None;
    }
    let (a, b) = (out.dims[1], out.dims[2]);
    if out.data.len() != a * b {
        return None;
    }
    match spec.layout {
        Layout::ChannelsFirst => decode(&out.data, a, b, class, min_score),
        Layout::AnchorsFirstRaw => decode_yolox(&out.data, a, b, class, min_score, spec.input_w, spec.input_h),
        Layout::AnchorsFirstDecoded => decode_decoded(&out.data, a, b, class, min_score),
    }
}

// YOLOX: the cells of the strides 8, 16, 32 in this order, row after row; offsets and log sizes -> pixels; score = objectness × class
fn decode_yolox(out: &[f32], anchors: usize, cols: usize, class: usize, min_score: f32, w: usize, h: usize) -> Option<Vec<Det>> {
    if cols < 6 || 5 + class >= cols || anchors != yolox_anchor_count(w, h) {
        return None;
    }
    let mut v = Vec::new();
    let mut i = 0;
    for stride in STRIDES {
        let (gw, gh) = (w / stride, h / stride);
        for gy in 0..gh {
            for gx in 0..gw {
                let row = &out[i * cols..(i + 1) * cols];
                i += 1;
                let score = row[4] * row[5 + class];
                if !score.is_finite() || score < min_score {
                    continue;
                }
                let s = stride as f32;
                // (an exp of a big number is not a box: a broken answer is skipped, not turned into infinity)
                if !(row[0].is_finite() && row[1].is_finite() && row[2].is_finite() && row[3].is_finite()) || row[2] > 12.0 || row[3] > 12.0 {
                    continue;
                }
                let (cx, cy) = ((row[0] + gx as f32) * s, (row[1] + gy as f32) * s);
                let (bw, bh) = (row[2].exp() * s, row[3].exp() * s);
                if bw <= 0.0 || bh <= 0.0 {
                    continue;
                }
                v.push(Det { x1: cx - bw / 2.0, y1: cy - bh / 2.0, x2: cx + bw / 2.0, y2: cy + bh / 2.0, score });
            }
        }
    }
    Some(v)
}

// YOLOv5: [cx, cy, w, h, objectness, classes…] in pixels
fn decode_decoded(out: &[f32], anchors: usize, cols: usize, class: usize, min_score: f32) -> Option<Vec<Det>> {
    if cols < 6 || 5 + class >= cols || anchors == 0 {
        return None;
    }
    let mut v = Vec::new();
    for i in 0..anchors {
        let row = &out[i * cols..(i + 1) * cols];
        let score = row[4] * row[5 + class];
        if !score.is_finite() || score < min_score || !(row[0].is_finite() && row[1].is_finite() && row[2].is_finite() && row[3].is_finite()) || row[2] <= 0.0 || row[3] <= 0.0 {
            continue;
        }
        v.push(Det { x1: row[0] - row[2] / 2.0, y1: row[1] - row[3] / 2.0, x2: row[0] + row[2] / 2.0, y2: row[1] + row[3] / 2.0, score });
    }
    Some(v)
}

// A square picture of the screen (BGRA, `side`×`side`) -> the planes R, G, B of the input of the model (`w`×`h`), with the colour numbers
// the model expects. When the sizes differ the picture is scaled (bilinear); the crop is square, a model with a non-square input gets
// the picture stretched (the boxes are scaled back by `scale_back` the same way).
pub fn preprocess(bgra: &[u8], side: usize, w: usize, h: usize, norm: Norm) -> Vec<f32> {
    let k = match norm {
        Norm::Unit => 1.0 / 255.0,
        Norm::Raw255 => 1.0,
    };
    let plane = w * h;
    let mut rgb = vec![0f32; plane * 3];
    if side == w && side == h {
        for i in 0..plane {
            rgb[i] = bgra[i * 4 + 2] as f32 * k;
            rgb[plane + i] = bgra[i * 4 + 1] as f32 * k;
            rgb[2 * plane + i] = bgra[i * 4] as f32 * k;
        }
        return rgb;
    }
    let (sx, sy) = (side as f32 / w as f32, side as f32 / h as f32);
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (side - 1) as f32);
        let (y0, wy) = (fy.floor() as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(side - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (side - 1) as f32);
            let (x0, wx) = (fx.floor() as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(side - 1);
            let at = |xx: usize, yy: usize, c: usize| bgra[(yy * side + xx) * 4 + c] as f32;
            for (plane_i, c) in [(0usize, 2usize), (1, 1), (2, 0)] {
                let top = at(x0, y0, c) * (1.0 - wx) + at(x1, y0, c) * wx;
                let bottom = at(x0, y1, c) * (1.0 - wx) + at(x1, y1, c) * wx;
                rgb[plane_i * plane + y * w + x] = (top * (1.0 - wy) + bottom * wy) * k;
            }
        }
    }
    rgb
}

// the boxes of the input of the model -> the pixels of the picture of the screen
pub fn scale_back(dets: Vec<Det>, side: usize, w: usize, h: usize) -> Vec<Det> {
    let (sx, sy) = (side as f32 / w.max(1) as f32, side as f32 / h.max(1) as f32);
    dets.into_iter().map(|d| Det { x1: d.x1 * sx, y1: d.y1 * sy, x2: d.x2 * sx, y2: d.y2 * sy, score: d.score }).collect()
}

// ---------------------------------------------------------------------------------------------
// ONNX Runtime: the same on every system that has it (the tests on Linux run the real runtime; the capture of the screen is Windows only)
// ---------------------------------------------------------------------------------------------
// a real model is megabytes; a file under this is not one (the tests use tiny fake models, so the bound is lower there)
pub const MIN_MODEL_BYTES: u64 = if cfg!(test) { 100 } else { 10_000 };

#[cfg(feature = "ort")]
pub mod ortrun {
    use super::*;
    use crate::aimassist::{AimConfig, Device};
    use ort::session::Session;
    use ort::tensor::TensorElementType;
    use ort::value::{Tensor, ValueType};

    // what is true of a model that loaded and ran once on a blank picture
    #[derive(Clone, Debug, PartialEq)]
    pub struct ModelInfo {
        pub input_name: String,
        pub spec: ModelSpec,
        pub output_dims: Vec<usize>,
        // one run on a blank picture, milliseconds (the first run is the slowest)
        pub trial_ms: u64,
        pub device: Device,
        // «DirectML is not in this build: CPU is used» and the like
        pub device_note: String,
    }

    // which device runs the model: Auto and CPU run on the processor; DirectML only in a build that has it (the feature `directml`)
    pub fn resolve_device(wanted: Device) -> (Device, String) {
        match wanted {
            Device::Cpu => (Device::Cpu, String::new()),
            Device::Auto if !cfg!(feature = "directml") => (Device::Cpu, "Auto: в этой сборке видеокарта (DirectML) не подключена, работает процессор".into()),
            Device::DirectMl if !cfg!(feature = "directml") => (Device::Cpu, "DirectML в этой сборке не подключён: работает процессор".into()),
            other => (other, String::new()),
        }
    }

    // a session of the model on the chosen device; a failure of DirectML is not fatal: the processor takes over, and the note says so
    pub fn session_for(path: &std::path::Path, wanted: Device) -> Result<(Session, Device, String), String> {
        let (device, mut note) = resolve_device(wanted);
        let _ = device;
        // (the program shares the computer with a game: the model gets a few threads, not all of them)
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 4);
        let base = || -> Result<ort::session::builder::SessionBuilder, String> {
            Session::builder().map_err(|e| format!("ONNX Runtime: {}", e))?.with_intra_threads(threads).map_err(|e| format!("ONNX Runtime: {}", e))
        };
        #[cfg(feature = "directml")]
        if device == Device::DirectMl || device == Device::Auto {
            let attempt = base().and_then(|b| b.with_execution_providers([ort::ep::DirectML::default().build()]).map_err(|e| e.to_string())).and_then(|b| b.commit_from_file(path).map_err(|e| e.to_string()));
            match attempt {
                Ok(s) => return Ok((s, Device::DirectMl, note)),
                Err(e) => note = format!("DirectML не запустился ({}): работает процессор", e),
            }
        }
        let _ = &mut note;
        let session = base()?.commit_from_file(path).map_err(|e| format!("модель не загрузилась: {}", e))?;
        Ok((session, Device::Cpu, note))
    }

    // the input of the model: its name and its size (0 = dynamic); a model with another kind of input is refused
    pub fn input_of(session: &Session) -> Result<(String, usize, usize), String> {
        let input = session.inputs().first().ok_or("у модели нет входа")?;
        match input.dtype() {
            ValueType::Tensor { ty: TensorElementType::Float32, shape, .. } if shape.len() == 4 && (shape[1] == 3 || shape[1] < 0) => {
                let dim = |i: usize| if shape[i] > 0 { shape[i] as usize } else { 0 };
                Ok((input.name().to_string(), dim(3), dim(2)))
            }
            other => Err(format!("вход модели не подходит: нужен тензор float32 1×3×высота×ширина, а там {:?}", other)),
        }
    }

    // one run on an RGB picture (the planes), the answer of the model
    pub fn run(session: &mut Session, name: &str, rgb: &[f32], w: usize, h: usize) -> Result<RawOut, String> {
        let arr = ndarray::Array4::from_shape_vec((1, 3, h, w), rgb.to_vec()).map_err(|e| e.to_string())?;
        let input = Tensor::from_array(arr).map_err(|e| format!("tensor: {}", e))?;
        let outputs = session.run(ort::inputs![name => input]).map_err(|e| format!("ONNX Runtime: {}", e))?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| format!("ответ модели не число float32: {}", e))?;
        Ok(RawOut { data: data.to_vec(), dims: shape.iter().map(|&d| d.max(0) as usize).collect() })
    }

    // The whole check of a model file: it loads, the input fits, a trial run on a blank picture gives an answer of a detector with the class
    // of a person. `side` is the size of the input when the model leaves it free. Nothing is changed on the disk.
    pub fn probe(path: &std::path::Path, cfg: &AimConfig) -> Result<ModelInfo, String> {
        let meta = std::fs::metadata(path).map_err(|_| "файл модели не найден".to_string())?;
        if !meta.is_file() || meta.len() < MIN_MODEL_BYTES {
            return Err("файл слишком маленький для модели".into());
        }
        if meta.len() > 1_500_000_000 {
            return Err("файл слишком большой для модели (больше 1,5 ГБ)".into());
        }
        let (mut session, device, note) = session_for(path, cfg.device)?;
        let (name, mut w, mut h) = input_of(&session)?;
        let free = cfg.side.max(64) as usize;
        if w == 0 {
            w = free;
        }
        if h == 0 {
            h = free;
        }
        if w > 2048 || h > 2048 || w < 64 || h < 64 {
            return Err(format!("размер входа модели {}×{} не подходит (от 64 до 2048)", w, h));
        }
        // a blank grey picture, in the numbers of either family: only the shape of the answer matters here
        let blank = vec![0.45f32; 3 * w * h];
        let started = std::time::Instant::now();
        let out = run(&mut session, &name, &blank, w, h)?;
        let trial_ms = started.elapsed().as_millis() as u64;
        let (layout, classes, anchors) = classify_output(&out.dims, w, h)?;
        if cfg.class >= classes {
            return Err(format!("в модели {} классов, а класс человека — номер {}: такой модели прицел не подходит", classes, cfg.class));
        }
        if out.data.iter().any(|v| !v.is_finite()) {
            return Err("модель на пустой картинке дала нечисловые значения (NaN): она не годится".into());
        }
        Ok(ModelInfo { input_name: name, spec: ModelSpec { layout, input_w: w, input_h: h, classes, anchors }, output_dims: out.dims, trial_ms, device, device_note: note })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(layout: Layout, classes: usize, anchors: usize) -> ModelSpec {
        ModelSpec { layout, input_w: 640, input_h: 640, classes, anchors }
    }

    #[test]
    fn the_shape_of_the_output_tells_the_family_of_the_model_and_a_classifier_is_refused() {
        // YOLOv8 with 80 classes
        assert_eq!(classify_output(&[1, 84, 8400], 640, 640).unwrap(), (Layout::ChannelsFirst, 80, 8400));
        // YOLOX raw: the number of anchors is the number of cells of the three strides
        assert_eq!(yolox_anchor_count(640, 640), 8400);
        assert_eq!(classify_output(&[1, 8400, 85], 640, 640).unwrap(), (Layout::AnchorsFirstRaw, 80, 8400));
        // YOLOv5 (boxes in pixels): the same shape with another number of anchors
        assert_eq!(classify_output(&[1, 25200, 85], 640, 640).unwrap(), (Layout::AnchorsFirstDecoded, 80, 25200));
        // not a detector
        for bad in [&[1usize, 1000][..], &[1, 3, 224, 224], &[2, 84, 8400], &[1, 4, 8400], &[1, 8400, 5], &[1, 0, 10], &[]] {
            assert!(classify_output(bad, 640, 640).is_err(), "{:?}", bad);
        }
        assert!(classify_output(&[1, 1000], 640, 640).unwrap_err().contains("не похожа на детектор"));
    }

    #[test]
    fn yolov8_boxes_yolox_cells_and_yolov5_pixels_each_give_the_right_box() {
        // YOLOv8: the same box in the layout of the channels
        let v8 = RawOut { data: vec![320.0, 330.0, 300.0, 302.0, 100.0, 100.0, 200.0, 200.0, 0.9, 0.8, 0.9, 0.0, 0.0, 0.7], dims: vec![1, 7, 2] };
        let s8 = spec(Layout::ChannelsFirst, 3, 2);
        let d = decode_output(&v8, &s8, 0, 0.5).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], Det { x1: 270.0, y1: 200.0, x2: 370.0, y2: 400.0, score: 0.9 });
        // the second class is another score row
        assert_eq!(decode_output(&v8, &s8, 2, 0.5).unwrap().len(), 1);
        // YOLOX: a person in the cell (40, 30) of the stride 8: the centre (320, 240), the size 100×200
        let mut data = vec![0f32; 8400 * 85];
        let i = 30 * 80 + 40;
        data[i * 85 + 2] = (100.0f32 / 8.0).ln();
        data[i * 85 + 3] = (200.0f32 / 8.0).ln();
        data[i * 85 + 4] = 0.9;
        data[i * 85 + 5] = 0.95;
        // another one in a cell of the stride 16: the cell (10, 5) -> the index 6400 + 5*40 + 10, centre ((0.5+10)*16, (0.5+5)*16) = (168, 88)
        let j = 6400 + 5 * 40 + 10;
        data[j * 85] = 0.5;
        data[j * 85 + 1] = 0.5;
        data[j * 85 + 2] = (64.0f32 / 16.0).ln();
        data[j * 85 + 3] = (128.0f32 / 16.0).ln();
        data[j * 85 + 4] = 0.8;
        data[j * 85 + 5] = 0.9;
        let yx = RawOut { data, dims: vec![1, 8400, 85] };
        let sx = spec(Layout::AnchorsFirstRaw, 80, 8400);
        let mut found = decode_output(&yx, &sx, 0, 0.3).unwrap();
        found.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        assert_eq!(found.len(), 2, "the empty cells have no objectness");
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(found[0].x1, 270.0) && close(found[0].y1, 140.0) && close(found[0].x2, 370.0) && close(found[0].y2, 340.0), "{:?}", found[0]);
        assert!(close(found[0].score, 0.855), "objectness × class: {}", found[0].score);
        assert!(close(found[1].x1, 168.0 - 32.0) && close(found[1].y1, 88.0 - 64.0) && close(found[1].x2, 168.0 + 32.0) && close(found[1].y2, 88.0 + 64.0), "{:?}", found[1]);
        // a broken answer: an infinite size is skipped, a wrong number of anchors is refused
        let mut broken = yx.clone();
        broken.data[i * 85 + 2] = 500.0;
        assert_eq!(decode_output(&broken, &sx, 0, 0.3).unwrap().len(), 1);
        assert!(decode_output(&RawOut { data: vec![0.0; 85 * 100], dims: vec![1, 100, 85] }, &sx, 0, 0.3).is_none());
        // YOLOv5: boxes in pixels
        let v5 = RawOut { data: vec![320.0, 240.0, 100.0, 200.0, 0.9, 0.95, 5.0, 5.0, 1.0, 1.0, 0.1, 0.1], dims: vec![1, 2, 6] };
        let d5 = decode_output(&v5, &spec(Layout::AnchorsFirstDecoded, 1, 2), 0, 0.3).unwrap();
        assert_eq!(d5.len(), 1);
        assert!(close(d5[0].y1, 140.0) && close(d5[0].score, 0.855));
        // a class that the model does not have
        assert!(decode_output(&v5, &spec(Layout::AnchorsFirstDecoded, 1, 2), 1, 0.3).is_none());
    }

    #[test]
    fn the_picture_becomes_planes_in_the_numbers_of_the_family_and_is_scaled_without_losing_the_places() {
        // 2×2: B, G, R, A of four pixels
        let frame = vec![0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 10, 20, 30, 255];
        let unit = preprocess(&frame, 2, 2, 2, Norm::Unit);
        assert!(unit[3] > 0.11764 && unit[3] < 0.117648 && unit[0] == 1.0, "the red plane {:?}", &unit[0..4]);
        assert!((unit[7] - 20.0 / 255.0).abs() < 1e-6 && unit[5] == 1.0, "the green plane");
        let raw = preprocess(&frame, 2, 2, 2, Norm::Raw255);
        assert_eq!(&raw[0..4], &[255.0, 0.0, 0.0, 30.0], "YOLOX takes 0..255");
        // scaling up 4×4 -> 8×8: a picture with a white square in the corner keeps it in the corner
        let mut big = vec![0u8; 4 * 4 * 4];
        for y in 0..2 {
            for x in 0..2 {
                for c in 0..3 {
                    big[(y * 4 + x) * 4 + c] = 255;
                }
            }
        }
        let up = preprocess(&big, 4, 8, 8, Norm::Unit);
        assert!(up[0] > 0.99 && up[7 * 8 + 7] < 0.01, "the white corner stays at the top left");
        assert!(up[8 * 8 * 2 + 0] > 0.99);
        // scaling down 640 -> 320 keeps the size of the planes
        let down = preprocess(&vec![100u8; 640 * 640 * 4], 640, 320, 320, Norm::Unit);
        assert_eq!(down.len(), 3 * 320 * 320);
        assert!((down[1000] - 100.0 / 255.0).abs() < 1e-5);
        // the boxes go back to the pixels of the screen
        let back = scale_back(vec![Det { x1: 100.0, y1: 100.0, x2: 200.0, y2: 300.0, score: 0.5 }], 320, 640, 640);
        assert_eq!((back[0].x1, back[0].y2), (50.0, 150.0));
    }

    // ---- the real ONNX Runtime with small fake models (a fixed answer): the whole way from the file to the boxes ----
    #[cfg(feature = "ort")]
    mod real {
        use super::*;
        use crate::aimassist::AimConfig;

        fn fixture(name: &str) -> std::path::PathBuf {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata").join(name)
        }

        // the fake files are small (a real model is tens of megabytes): the probe is told to accept them
        fn cfg() -> AimConfig {
            AimConfig::default()
        }

        #[test]
        fn the_probe_understands_each_family_and_tells_why_a_file_does_not_fit() {
            let x = ortrun::probe(&fixture("aim_yolox_dummy.onnx"), &cfg()).unwrap();
            assert_eq!((x.spec.layout, x.spec.classes, x.spec.anchors, x.spec.input_w, x.spec.input_h), (Layout::AnchorsFirstRaw, 80, 8400, 640, 640));
            assert_eq!((x.input_name.as_str(), x.device), ("images", crate::aimassist::Device::Cpu));
            let v8 = ortrun::probe(&fixture("aim_dummy.onnx"), &cfg()).unwrap();
            assert_eq!((v8.spec.layout, v8.spec.classes), (Layout::ChannelsFirst, 1));
            assert_eq!(v8.spec.input_w, 640, "a model that leaves the size free gets the size of the settings");
            let err = ortrun::probe(&fixture("aim_classifier_dummy.onnx"), &cfg()).unwrap_err();
            assert!(err.contains("не похожа на детектор"), "{}", err);
            // a model with fewer classes than the class of a person needs
            let one_class = AimConfig { class: 5, ..cfg() };
            assert!(ortrun::probe(&fixture("aim_dummy.onnx"), &one_class).unwrap_err().contains("класс человека"));
        }

        #[test]
        fn the_session_runs_on_the_processor_and_a_wish_for_directml_falls_back_to_it_with_a_note() {
            use crate::aimassist::Device;
            let (d, note) = ortrun::resolve_device(Device::Cpu);
            assert_eq!((d, note.is_empty()), (Device::Cpu, true));
            let (d, note) = ortrun::resolve_device(Device::DirectMl);
            if cfg!(feature = "directml") {
                assert_eq!(d, Device::DirectMl);
            } else {
                assert_eq!(d, Device::Cpu);
                assert!(note.contains("DirectML"), "{}", note);
            }
            let (d, _) = ortrun::resolve_device(Device::Auto);
            assert!(matches!(d, Device::Cpu | Device::DirectMl));
            let (mut s, used, _) = ortrun::session_for(&fixture("aim_yolox_dummy.onnx"), Device::DirectMl).unwrap();
            assert_eq!(used, Device::Cpu);
            let (name, w, h) = ortrun::input_of(&s).unwrap();
            assert_eq!((name.as_str(), w, h), ("images", 640, 640));
            let out = ortrun::run(&mut s, &name, &vec![0.0; 3 * 640 * 640], 640, 640).unwrap();
            assert_eq!(out.dims, vec![1, 8400, 85]);
        }

        #[test]
        fn the_fake_yolox_model_gives_the_person_at_the_right_place_through_the_real_runtime() {
            let (mut s, _, _) = ortrun::session_for(&fixture("aim_yolox_dummy.onnx"), crate::aimassist::Device::Cpu).unwrap();
            let (name, w, h) = ortrun::input_of(&s).unwrap();
            let out = ortrun::run(&mut s, &name, &vec![1.0; 3 * w * h], w, h).unwrap();
            let (layout, classes, anchors) = classify_output(&out.dims, w, h).unwrap();
            assert_eq!((layout, classes, anchors), (Layout::AnchorsFirstRaw, 80, 8400));
            let spec = ModelSpec { layout, input_w: w, input_h: h, classes, anchors };
            let dets = decode_output(&out, &spec, 0, 0.4).unwrap();
            assert_eq!(dets.len(), 1);
            let d = dets[0];
            assert!((d.x1 - 270.0).abs() < 0.5 && (d.y1 - 140.0).abs() < 0.5 && (d.x2 - 370.0).abs() < 0.5 && (d.y2 - 340.0).abs() < 0.5, "{:?}", d);
        }

        #[test]
        fn a_classifier_is_refused_for_its_answer_not_for_its_name() {
            let (mut s, _, _) = ortrun::session_for(&fixture("aim_classifier_dummy.onnx"), crate::aimassist::Device::Cpu).unwrap();
            let (name, w, h) = ortrun::input_of(&s).unwrap();
            assert_eq!((w, h), (224, 224));
            let out = ortrun::run(&mut s, &name, &vec![0.0; 3 * 224 * 224], 224, 224).unwrap();
            assert!(classify_output(&out.dims, w, h).unwrap_err().contains("не похожа на детектор"));
        }

        #[test]
        fn a_file_that_is_not_a_model_does_not_load() {
            let junk = std::env::temp_dir().join(format!("jarvis-aimdetect-junk-{}.onnx", std::process::id()));
            std::fs::write(&junk, vec![7u8; 50_000]).unwrap();
            assert!(ortrun::probe(&junk, &cfg()).unwrap_err().contains("не загрузилась"));
            assert!(ortrun::probe(&std::env::temp_dir().join("нет-такой-модели.onnx"), &cfg()).unwrap_err().contains("не найден"));
            std::fs::remove_file(&junk).ok();
        }
    }
}
