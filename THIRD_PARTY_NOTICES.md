# Third-party components

Stutter Watch is offered under the root [PolyForm Noncommercial License 1.0.0](LICENSE). Dependencies retain their own copyright and license terms. Cargo downloads dependency sources; they are not vendored in this repository. Exact versions are recorded in [Cargo.lock](Cargo.lock).

Direct runtime dependencies, checked against the locked Cargo metadata:

| Component | Version | Upstream license |
| --- | --- | --- |
| [anyhow](https://github.com/dtolnay/anyhow) | 1.0.105 | MIT OR Apache-2.0 |
| [chrono](https://github.com/chronotope/chrono) | 0.4.45 | MIT OR Apache-2.0 |
| [clap](https://github.com/clap-rs/clap) | 4.6.10 | MIT OR Apache-2.0 |
| [lz4_flex](https://github.com/pseitz/lz4_flex) | 0.14.2 | MIT |
| [nvml-wrapper](https://github.com/Cldfire/nvml-wrapper) | 0.11.0 | MIT OR Apache-2.0 |
| [serde](https://github.com/serde-rs/serde) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_json](https://github.com/serde-rs/json) | 1.0.154 | MIT OR Apache-2.0 |
| [toml](https://github.com/toml-rs/toml) | 0.9.12+spec-1.1.0 | MIT OR Apache-2.0 |
| [windows](https://github.com/microsoft/windows-rs) | 0.62.2 | MIT OR Apache-2.0 |

`OR` identifies an upstream choice of license, not a requirement to apply both. The development-only compression probe also uses `flate2` and `zstd`; test helpers use `tempfile`. Windows release archives include a target-specific normal/build dependency inventory in `licenses/dependencies.json`, the upstream notices for every listed package, and Rust library copyright/license files in `licenses/rust`. The packaging script stops if a package has no license declaration or notice file. Windows system libraries and NVIDIA drivers/NVML are supplied separately by their vendors; they are not bundled here.

The compression notices below are retained for convenience. No source files from the reference monitor repositories were copied.


## lz4_flex-0.14.2

The MIT License (MIT)

Copyright (c) 2020 Pascal Seitz

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.


## twox-hash-2.1.5

The MIT License (MIT)

Copyright (c) 2015 Jake Goulding

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
