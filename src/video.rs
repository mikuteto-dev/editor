use std::{path::Path, ptr::NonNull};

use gpui_kit::*;
use objc2::{AnyThread, MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_av_foundation::{AVPlayer, AVPlayerItemVideoOutput};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_core_video::{
    CVPixelBuffer, kCVPixelBufferMetalCompatibilityKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

pub struct Video {
    player: Retained<AVPlayer>,
    output: Retained<AVPlayerItemVideoOutput>,
    frame: Option<CFRetained<CVPixelBuffer>>,
    playing: bool,
}

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["mp4", "mov", "m4v"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// CoreVideo keys are CFStrings, toll-free bridged to NSString.
fn key(s: &CFString) -> &NSString {
    unsafe { &*(s as *const CFString as *const NSString) }
}

impl Video {
    pub fn new(path: &Path) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        // GPUI's surface pipeline reads biplanar 4:2:0 buffers through Metal.
        let format = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange);
        let metal = NSNumber::new_bool(true);
        unsafe {
            let attributes = NSDictionary::<NSString, AnyObject>::from_slices(
                &[key(kCVPixelBufferPixelFormatTypeKey), key(kCVPixelBufferMetalCompatibilityKey)],
                &[format.as_ref(), metal.as_ref()],
            );
            let player = AVPlayer::playerWithURL(&url, mtm);
            let output = AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                AVPlayerItemVideoOutput::alloc(),
                Some(&attributes),
            );
            player.currentItem()?.addOutput(&output);
            player.play();
            Some(Self {
                player,
                output,
                frame: None,
                playing: true,
            })
        }
    }

    pub fn toggle(&mut self) {
        self.set_playing(!self.playing);
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        unsafe {
            if playing {
                self.player.play()
            } else {
                self.player.pause()
            }
        }
    }

    // Frames are requested only while playing, so a paused video costs nothing.
    pub fn render(&mut self, window: &mut Window) -> Option<Surface> {
        unsafe {
            let time = self.player.currentTime();
            if self.output.hasNewPixelBufferForItemTime(time)
                && let Some(buffer) = self
                    .output
                    .copyPixelBufferForItemTime_itemTimeForDisplay(time, std::ptr::null_mut())
            {
                // Same +1 reference, moved from objc2's Retained into the CF wrapper gpui takes.
                let raw = NonNull::new(Retained::into_raw(buffer)).expect("non-null buffer");
                self.frame = Some(CFRetained::from_raw(raw));
            }
        }
        if self.playing {
            window.request_animation_frame();
        }
        self.frame.clone().map(|frame| surface(frame).size_full())
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        unsafe { self.player.pause() };
    }
}
