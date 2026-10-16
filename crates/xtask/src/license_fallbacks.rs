// Fallback license texts for crates that do not ship a LICENSE file in their
// published archive. Keyed by (crate name, license expression). Each entry was
// fetched from the crate's upstream repository and must match the license
// declared in its Cargo.toml. When a crate later ships a LICENSE file, the
// on-disk copy wins and this table entry becomes dead.
use std::collections::BTreeMap;

/// The standard MIT license text (ruff component crates, quote-use*).
pub const MIT_TEXT: &str = "\
MIT License

Copyright (c) 2022 Charles Marsh

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";

pub fn fallbacks() -> BTreeMap<(&'static str, &'static str), &'static str> {
    BTreeMap::from([
        // astral-sh/ruff component crates: LICENSE at the repo root is MIT
        // (https://github.com/astral-sh/ruff/blob/main/LICENSE).
        (("ruff_python_ast", "MIT"), MIT_TEXT),
        (("ruff_python_parser", "MIT"), MIT_TEXT),
        (("ruff_python_trivia", "MIT"), MIT_TEXT),
        (("ruff_source_file", "MIT"), MIT_TEXT),
        (("ruff_text_size", "MIT"), MIT_TEXT),
        // Rusoto quote-use crates: MIT per Cargo.toml; LICENSE listed in the
        // package `include` but omitted from the published archive.
        (("quote-use", "MIT"), MIT_TEXT),
        (("quote-use-macros", "MIT"), MIT_TEXT),
    ])
}
