// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.2 built-in codecs (PNG/JPEG/TIFF/WebP/GIF/BMP); DD ⑫.
//! Each submodule implements the §8.1 [`crate::registry::Codec`] trait for
//! one format; shared decode plumbing (limits, orientation, metadata
//! carriage) lives in [`shared`].

pub mod bmp;
pub mod gif_io;
pub mod jpeg;
pub mod ora;
pub mod png_wrap;
pub mod psd;
pub mod shared;
pub mod tiff_io;
pub mod webp;
