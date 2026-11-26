use settings::{RegisterSetting, Settings};
use settings_content::VoiceInputSettingsContent;

/// Resolved voice input settings.
#[derive(Clone, Debug, Default, RegisterSetting)]
pub struct VoiceInputSettings {
    content: VoiceInputSettingsContent,
}

impl VoiceInputSettings {
    pub fn content(&self) -> &VoiceInputSettingsContent {
        &self.content
    }
}

impl Settings for VoiceInputSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            content: content.voice_input.clone().unwrap_or_default(),
        }
    }
}
