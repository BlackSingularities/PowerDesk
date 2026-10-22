//! Strony PDF renderowane przez Windows (Windows.Data.Pdf) – bez dodatkowych bibliotek.
//! Wywołania blokują, więc tylko z wątku w tle.

use windows::core::HSTRING;
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

fn open(path: &str) -> windows::core::Result<PdfDocument> {
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.get()?;
    PdfDocument::LoadFromFileAsync(&file)?.get()
}

/// Liczba stron (None – plik zaszyfrowany albo uszkodzony).
pub fn page_count(path: &str) -> Option<u32> {
    open(path).and_then(|d| d.PageCount()).ok()
}

/// Strona `index` (od 0) jako PNG o szerokości `width` pikseli; zwraca też liczbę stron.
pub fn render(path: &str, index: u32, width: u32) -> Option<(Vec<u8>, u32)> {
    let run = || -> windows::core::Result<(Vec<u8>, u32)> {
        let doc = open(path)?;
        let count = doc.PageCount()?;
        let page = doc.GetPage(index.min(count.saturating_sub(1)))?;
        let opts = PdfPageRenderOptions::new()?;
        opts.SetDestinationWidth(width.max(1))?;
        let stream = InMemoryRandomAccessStream::new()?;
        page.RenderWithOptionsToStreamAsync(&stream, &opts)?.get()?;
        let size = stream.Size()? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.get()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf)?;
        Ok((buf, count))
    };
    run().ok()
}
