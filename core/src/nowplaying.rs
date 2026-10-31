//! Co teraz gra: systemowe sterowanie multimediami Windows (to samo, co pokazuje okienko
//! głośności) – tytuł, wykonawca, stan i przyciski odtwarzania dowolnego programu.

use windows::Media::Control::{GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playing {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Program, który gra (identyfikator aplikacji).
    pub app: String,
}

fn session() -> Option<Session> {
    Manager::RequestAsync().ok()?.get().ok()?.GetCurrentSession().ok()
}

/// Bieżące multimedia albo None, gdy nic nie gra i nic nie jest wstrzymane.
pub fn current() -> Option<Playing> {
    let s = session()?;
    let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let playing = s.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok()).is_some_and(|st| st == Status::Playing);
    let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    Some(Playing { title, artist: props.Artist().map(|t| t.to_string()).unwrap_or_default(), playing, app: s.SourceAppUserModelId().map(|t| t.to_string()).unwrap_or_default() })
}

#[derive(Clone, Copy, Debug)]
pub enum Cmd {
    Previous,
    PlayPause,
    Next,
}

/// Przycisk odtwarzania – w bieżącym programie.
pub fn command(c: Cmd) {
    let Some(s) = session() else { return };
    let r = match c {
        Cmd::Previous => s.TrySkipPreviousAsync(),
        Cmd::PlayPause => s.TryTogglePlayPauseAsync(),
        Cmd::Next => s.TrySkipNextAsync(),
    };
    if let Ok(op) = r {
        let _ = op.get();
    }
}

/// Okładka bieżącego utworu (zakodowany obraz, zwykle PNG albo JPEG) – wołać z wątku w tle.
pub fn cover() -> Option<Vec<u8>> {
    use windows::Storage::Streams::DataReader;
    let s = session()?;
    let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 16 << 20 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).ok()?;
    Some(buf)
}
