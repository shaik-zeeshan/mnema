# OCR provider benchmarks

The benchmark examples run each OCR provider against the bundled fixtures copied from:

- `/Users/shaikzeeshan/Downloads/high-quality.jpg`
- `/Users/shaikzeeshan/Downloads/low-quality.jpg`

Examples:

```sh
cargo run -p ocr --example apple_vision_benchmark -- --iterations 10
cargo run -p ocr --features tesseract-embedded --example tesseract_benchmark -- --models-dir /path/to/ocr-models
cargo run -p ocr --features paddle-rs --example paddle_ocr_benchmark -- --models-dir /path/to/ocr-models
```

Use `--image label=/path/to/image.jpg` to add/replace inputs and `--model-path /path/to/provider/model` to bypass the default provider/model folder lookup.
