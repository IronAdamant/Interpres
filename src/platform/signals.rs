//! Versioned detection tables (plan L0). Editable without redesign.

#[derive(Clone, Debug)]
pub struct SignalTable {
    pub process_names: &'static [&'static str],
    pub window_classes: &'static [&'static str],
    pub text_automation_ids: &'static [&'static str],
    pub ignore_automation_ids: &'static [&'static str],
    pub bundle_ids: &'static [&'static str],
    pub process_substrings: &'static [&'static str],
}

pub fn windows_signals() -> SignalTable {
    SignalTable {
        process_names: &["LiveCaptions", "LiveCaptions.exe"],
        window_classes: &["LiveCaptionsDesktopWindow"],
        text_automation_ids: &["CaptionsTextBlock", "CaptionsScrollViewer"],
        ignore_automation_ids: &["ReadyToCaptionTextBlock"],
        bundle_ids: &[],
        process_substrings: &["LiveCaptions"],
    }
}

pub fn macos_signals() -> SignalTable {
    SignalTable {
        process_names: &["Live Captions"],
        window_classes: &[],
        text_automation_ids: &[],
        ignore_automation_ids: &[],
        bundle_ids: &["com.apple.accessibility.LiveTranscriptionAgent"],
        process_substrings: &["Live Captions", "LiveTranscriptionAgent"],
    }
}
