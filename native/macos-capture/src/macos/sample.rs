//! Conversions from CoreMedia/CoreVideo/CoreGraphics buffers to `capture_core` frames.

use crate::audio::{to_interleaved_f32, PcmLayout};
use crate::pixels::{fourcc_to_string, pack_bgra_rows, required_len, PIXEL_FORMAT_32BGRA};
use crate::timing::instant_from_age;
use capture_core::{AudioBuffer, CaptureError, CaptureResult, VideoFrame};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_audio_types::{AudioBuffer as CaAudioBuffer, AudioBufferList};
use objc2_core_foundation::{CFArray, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_core_media::{
    kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, CMAudioFormatDescriptionGetStreamBasicDescription,
    CMBlockBuffer, CMClock, CMSampleBuffer, CMTimeFlags,
};
use objc2_core_video::{
    kCVReturnSuccess, CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetDataSize,
    CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferIsPlanar,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use objc2_screen_capture_kit::{SCFrameStatus, SCStreamFrameInfoStatus};
use std::ptr::{self, NonNull};
use std::time::Instant;

/// Why a video sample could not be converted.
#[derive(Debug)]
pub(crate) enum FrameError {
    /// The stream delivers something other than BGRA: a configuration bug, fatal.
    Format(CaptureError),
    /// A single frame could not be read (lock failure etc.); skip it.
    Transient(CaptureError),
}

/// `SCStreamFrameInfoStatus` from the sample's attachment dictionary, if present.
///
/// # Safety
/// `sample` must be a valid sample buffer delivered by ScreenCaptureKit.
pub(crate) unsafe fn frame_status(sample: &CMSampleBuffer) -> Option<SCFrameStatus> {
    let attachments: CFRetained<CFArray> = unsafe { sample.sample_attachments_array(false) }?;
    // SAFETY: sample attachments are a CFArray of CFDictionaries with CFString keys; CF and
    // Foundation collections are toll-free bridged.
    let typed: &CFArray<NSDictionary<NSString, AnyObject>> = unsafe { attachments.cast_unchecked() };
    let array: &NSArray<NSDictionary<NSString, AnyObject>> = typed.as_ref();
    let dict = array.firstObject()?;
    // SAFETY: immutable NSString constant exported by ScreenCaptureKit.
    let key: &NSString = unsafe { SCStreamFrameInfoStatus };
    let value: Retained<AnyObject> = dict.objectForKey(key)?;
    let number = value.downcast::<NSNumber>().ok()?;
    Some(SCFrameStatus(number.integerValue()))
}

/// Best-effort capture `Instant` from the sample's presentation timestamp (host clock).
///
/// # Safety
/// `sample` must be a valid sample buffer.
pub(crate) unsafe fn captured_at(sample: &CMSampleBuffer) -> Instant {
    let now = Instant::now();
    let pts = unsafe { sample.presentation_time_stamp() };
    if !pts.flags.contains(CMTimeFlags::Valid) {
        return now;
    }
    // SAFETY: the host time clock is a process-wide singleton; CMTime math on valid values.
    let age = unsafe {
        let host_now = CMClock::host_time_clock().time();
        host_now.subtract(pts).seconds()
    };
    instant_from_age(now, age)
}

/// Unlocks a locked `CVPixelBuffer` base address when dropped.
struct BaseAddressLock<'a>(&'a CVPixelBuffer);

impl Drop for BaseAddressLock<'_> {
    fn drop(&mut self) {
        // SAFETY: constructed only after a successful read-only lock with the same flags.
        unsafe { CVPixelBufferUnlockBaseAddress(self.0, CVPixelBufferLockFlags::ReadOnly) };
    }
}

/// Copies a BGRA `CVPixelBuffer` into a tightly packed `VideoFrame`.
pub(crate) fn copy_bgra(pixel_buffer: &CVPixelBuffer, captured_at: Instant) -> Result<VideoFrame, FrameError> {
    let format = CVPixelBufferGetPixelFormatType(pixel_buffer);
    if format != PIXEL_FORMAT_32BGRA || CVPixelBufferIsPlanar(pixel_buffer) {
        return Err(FrameError::Format(CaptureError::internal(format!(
            "ScreenCaptureKit delivered pixel format '{}' instead of BGRA",
            fourcc_to_string(format)
        ))));
    }
    // SAFETY: read-only lock of a valid pixel buffer; balanced by `BaseAddressLock`.
    let rc = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, CVPixelBufferLockFlags::ReadOnly) };
    if rc != kCVReturnSuccess {
        return Err(FrameError::Transient(CaptureError::internal(format!("CVPixelBufferLockBaseAddress failed ({rc})"))));
    }
    let _lock = BaseAddressLock(pixel_buffer);
    let base = CVPixelBufferGetBaseAddress(pixel_buffer).cast::<u8>().cast_const();
    let width = CVPixelBufferGetWidth(pixel_buffer);
    let height = CVPixelBufferGetHeight(pixel_buffer);
    let bytes_per_row = CVPixelBufferGetBytesPerRow(pixel_buffer);
    let data_size = CVPixelBufferGetDataSize(pixel_buffer);
    if base.is_null() {
        return Err(FrameError::Transient(CaptureError::internal("CVPixelBuffer has no base address")));
    }
    let len = required_len(width, height, bytes_per_row)
        .ok_or_else(|| FrameError::Transient(CaptureError::internal("pixel buffer size overflows")))?;
    if data_size != 0 && data_size < len {
        return Err(FrameError::Transient(CaptureError::internal(format!(
            "CVPixelBuffer reports {data_size} bytes but {width}x{height} @ {bytes_per_row} needs {len}"
        ))));
    }
    // SAFETY: the base address is locked for reading and the buffer holds at least `len`
    // bytes (rows are `bytes_per_row` apart; checked against the reported data size).
    let src = unsafe { std::slice::from_raw_parts(base, len) };
    let packed = pack_bgra_rows(src, width, height, bytes_per_row).map_err(FrameError::Transient)?;
    drop(_lock);
    let mut frame = VideoFrame::new_bgra(width as u32, height as u32, (width * 4) as u32, packed).map_err(FrameError::Transient)?;
    frame.captured_at = captured_at;
    Ok(frame)
}

/// Converts an SCK audio sample buffer (normally non-interleaved Float32) to interleaved f32.
///
/// # Safety
/// `sample` must be a valid audio sample buffer delivered by ScreenCaptureKit.
pub(crate) unsafe fn audio_buffer(sample: &CMSampleBuffer) -> CaptureResult<Option<AudioBuffer>> {
    // SAFETY (whole function): `sample` is valid per the contract; every pointer we
    // dereference comes from CoreMedia and is checked for null first.
    unsafe {
        if !sample.is_valid() || !sample.data_is_ready() {
            return Ok(None);
        }
        let Some(desc) = sample.format_description() else { return Ok(None) };
        let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(&desc);
        let Some(asbd) = asbd.as_ref() else { return Ok(None) };
        let layout = PcmLayout::from_asbd(asbd.mFormatID, asbd.mFormatFlags, asbd.mBitsPerChannel, asbd.mChannelsPerFrame, asbd.mSampleRate)?;

        // 1) Ask how large the AudioBufferList has to be (one AudioBuffer per channel when
        //    non-interleaved).
        let mut needed: usize = 0;
        let status = sample.audio_buffer_list_with_retained_block_buffer(
            &mut needed,
            ptr::null_mut(),
            0,
            None,
            None,
            0,
            ptr::null_mut(),
        );
        // Fallback if the size query is refused: one AudioBuffer per channel always suffices.
        let per_channel =
            std::mem::offset_of!(AudioBufferList, mBuffers) + layout.channels as usize * std::mem::size_of::<CaAudioBuffer>();
        if status != 0 || needed < std::mem::size_of::<AudioBufferList>() {
            log::debug!("macos-capture: audio buffer list size query returned OSStatus {status} ({needed} bytes); using {per_channel}");
            needed = per_channel;
        }
        let needed = needed.max(per_channel);
        // 2) Fill it. u64 storage gives the 8-byte alignment AudioBufferList needs.
        let mut storage = vec![0u64; needed.div_ceil(8)];
        let storage_bytes = storage.len() * 8;
        let list = storage.as_mut_ptr().cast::<AudioBufferList>();
        let mut block: *mut CMBlockBuffer = ptr::null_mut();
        let status = sample.audio_buffer_list_with_retained_block_buffer(
            ptr::null_mut(),
            list,
            storage_bytes,
            None,
            None,
            kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            &mut block,
        );
        // We own the +1 block buffer that backs the AudioBuffer pointers; release it when
        // this function returns (after the samples were copied).
        let _block = NonNull::new(block).map(|b| CFRetained::from_raw(b));
        if status != 0 {
            return Err(CaptureError::internal(format!("CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer failed (OSStatus {status})")));
        }
        let count = (*list).mNumberBuffers as usize;
        let capacity = (storage_bytes - std::mem::offset_of!(AudioBufferList, mBuffers)) / std::mem::size_of::<CaAudioBuffer>();
        if count > capacity {
            return Err(CaptureError::internal(format!("audio buffer list claims {count} buffers, room for {capacity}")));
        }
        // `mBuffers` is a C flexible array: take a raw pointer to its start without creating a
        // reference to the 1-element Rust array, so we may index past the first element.
        let first = ptr::addr_of!((*list).mBuffers).cast::<CaAudioBuffer>();
        let mut planes: Vec<&[u8]> = Vec::with_capacity(count);
        for i in 0..count {
            let b = &*first.add(i);
            planes.push(if b.mData.is_null() || b.mDataByteSize == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(b.mData.cast::<u8>().cast_const(), b.mDataByteSize as usize)
            });
        }
        let samples = to_interleaved_f32(&layout, &planes)?;
        drop(planes);
        if samples.is_empty() {
            return Ok(None);
        }
        Ok(Some(AudioBuffer { sample_rate: layout.sample_rate_hz(), channels: layout.channels, samples, captured_at: captured_at(sample) }))
    }
}

/// Renders a `CGImage` (from `SCScreenshotManager`) into a BGRA sRGB `VideoFrame`.
pub(crate) fn cgimage_to_frame(image: &CGImage) -> CaptureResult<VideoFrame> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    if width == 0 || height == 0 || width > 32_768 || height > 32_768 {
        return Err(CaptureError::internal(format!("screenshot image has an invalid size {width}x{height}")));
    }
    let stride = width * 4;
    let mut data = vec![0u8; stride * height];
    // SAFETY: immutable CFString constant exported by CoreGraphics.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or_else(|| CaptureError::internal("could not create the sRGB color space"))?;
    // BGRA in memory = 32-bit little-endian words with alpha first (premultiplied).
    let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;
    {
        // SAFETY: `data` is `stride * height` bytes and outlives the context (dropped at the
        // end of this block, before `data` is moved into the frame).
        let ctx = unsafe { CGBitmapContextCreate(data.as_mut_ptr().cast(), width, height, 8, stride, Some(&space), info) }
            .ok_or_else(|| CaptureError::internal("CGBitmapContextCreate failed"))?;
        let rect = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: width as f64, height: height as f64 } };
        CGContext::draw_image(Some(&ctx), rect, Some(image));
    }
    VideoFrame::new_bgra(width as u32, height as u32, stride as u32, data)
}
