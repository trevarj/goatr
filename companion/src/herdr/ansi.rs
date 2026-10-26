// SPDX-License-Identifier: GPL-3.0-or-later
//! Inverse of installed pl77 Herdr's direct blitter, deliberately not a VT emulator.
use super::*;
use tokio::time::Instant;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

type Result<T, E = ErrorCode> = std::result::Result<T, E>;
fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(ErrorCode::NotReady)
    }
}
const PREFIX: &str = "\x1b[?2026h\x1b[?25l\x1b]8;;\x1b\\";
const END: &str = "\x1b[?2026l";
const MAX_CELLS: usize = 8192;
fn geometry(columns: u16, rows: u16) -> Result<()> {
    require(
        (1..=512).contains(&columns)
            && (1..=256).contains(&rows)
            && usize::from(columns) * usize::from(rows) <= MAX_CELLS,
    )
}
pub(super) fn viewport(columns: u16, rows: u16) -> (u16, u16) {
    let columns = columns.clamp(1, 512);
    (
        columns,
        rows.clamp(1, 256)
            .min((MAX_CELLS / usize::from(columns)) as u16),
    )
}
fn uint(bytes: &mut Vec<u8>, value: u64) {
    if value <= 250 {
        bytes.push(value as u8)
    } else if value <= u64::from(u16::MAX) {
        bytes.push(251);
        bytes.extend_from_slice(&(value as u16).to_le_bytes())
    } else if value <= u64::from(u32::MAX) {
        bytes.push(252);
        bytes.extend_from_slice(&(value as u32).to_le_bytes())
    } else {
        bytes.push(253);
        bytes.extend_from_slice(&value.to_le_bytes())
    }
}
fn framed(mut bytes: Vec<u8>) -> Vec<u8> {
    let length = (bytes.len() - 4) as u32;
    bytes[..4].copy_from_slice(&length.to_le_bytes());
    bytes
}
fn hello(columns: u16, rows: u16) -> Result<Vec<u8>> {
    geometry(columns, rows)?;
    let mut bytes = vec![0, 0, 0, 0, 0, 22];
    uint(&mut bytes, u64::from(columns));
    uint(&mut bytes, u64::from(rows));
    bytes.extend_from_slice(&[0, 0, 0]);
    Ok(framed(bytes))
}
fn observe(terminal: &Id) -> Result<Vec<u8>> {
    terminal.validate().map_err(|_| ErrorCode::InvalidRequest)?;
    let mut bytes = vec![0, 0, 0, 0, 7];
    uint(&mut bytes, terminal.0.len() as u64);
    bytes.extend_from_slice(terminal.0.as_bytes());
    Ok(framed(bytes))
}
fn attributes() -> GridAttributes {
    GridAttributes {
        bold: false,
        faint: false,
        italic: false,
        blink: false,
        strike: false,
        inverse: false,
        concealed: false,
        underline: GridUnderline::None,
        underline_color: None,
    }
}
fn blank() -> GridCell {
    GridCell {
        text: " ".into(),
        width: Uint(1),
        foreground: GridColor::Default,
        background: GridColor::Default,
        attributes: attributes(),
    }
}
fn number(text: &str, maximum: usize) -> Result<usize> {
    require(
        !text.is_empty()
            && text.len() <= 5
            && (text.len() == 1 || !text.starts_with('0'))
            && text.bytes().all(|b| b.is_ascii_digit()),
    )?;
    let value = text.parse::<usize>().map_err(|_| ErrorCode::NotReady)?;
    require(value <= maximum)?;
    Ok(value)
}
fn position(text: &str, columns: u16, rows: u16) -> Result<(usize, usize)> {
    let (row, column) = text.split_once(';').ok_or(ErrorCode::NotReady)?;
    let y = number(row, usize::from(rows))?;
    let x = number(column, usize::from(columns))?;
    require(x > 0 && y > 0)?;
    Ok((x - 1, y - 1))
}
fn color<'a>(parts: &mut impl Iterator<Item = &'a str>, foreground: bool) -> Result<GridColor> {
    let code = number(parts.next().ok_or(ErrorCode::NotReady)?, 107)?;
    match (foreground, code) {
        (true, 39) | (false, 49) => Ok(GridColor::Default),
        (true, 30..=37) => Ok(GridColor::Indexed {
            index: Uint((code - 30) as u64),
        }),
        (false, 40..=47) => Ok(GridColor::Indexed {
            index: Uint((code - 40) as u64),
        }),
        (true, 90..=97) => Ok(GridColor::Indexed {
            index: Uint((code - 90 + 8) as u64),
        }),
        (false, 100..=107) => Ok(GridColor::Indexed {
            index: Uint((code - 100 + 8) as u64),
        }),
        (true, 38) | (false, 48) => match parts.next().ok_or(ErrorCode::NotReady)? {
            "5" => Ok(GridColor::Indexed {
                index: Uint(number(parts.next().ok_or(ErrorCode::NotReady)?, 255)? as u64),
            }),
            "2" => {
                let mut rgb = 0;
                for _ in 0..3 {
                    rgb = rgb * 256 + number(parts.next().ok_or(ErrorCode::NotReady)?, 255)? as u64;
                }
                Ok(GridColor::Rgb { value: Uint(rgb) })
            }
            _ => Err(ErrorCode::NotReady),
        },
        _ => Err(ErrorCode::NotReady),
    }
}
fn style(text: &str) -> Result<Option<GridCell>> {
    if text == "0" {
        return Ok(None);
    }
    let mut parts = text.split(';').peekable();
    require(parts.next() == Some("0"))?;
    let mut cell = blank();
    let mut last = 0;
    while let Some(part) = parts.peek().copied() {
        let code = match part {
            "1" => 1,
            "2" => 2,
            "3" => 3,
            "4" | "4:2" | "4:3" | "4:4" | "4:5" => 4,
            "5" => 5,
            "6" => 6,
            "7" => 7,
            "8" => 8,
            "9" => 9,
            _ => break,
        };
        require(code > last)?;
        last = code;
        parts.next();
        match code {
            1 => cell.attributes.bold = true,
            2 => cell.attributes.faint = true,
            3 => cell.attributes.italic = true,
            4 => {
                cell.attributes.underline = match part {
                    "4:2" => GridUnderline::Double,
                    "4:3" => GridUnderline::Curly,
                    "4:4" => GridUnderline::Dotted,
                    "4:5" => GridUnderline::Dashed,
                    _ => GridUnderline::Single,
                }
            }
            5 | 6 => cell.attributes.blink = true,
            7 => cell.attributes.inverse = true,
            8 => cell.attributes.concealed = true,
            9 => cell.attributes.strike = true,
            _ => unreachable!(),
        }
    }
    cell.foreground = color(&mut parts, true)?;
    cell.background = color(&mut parts, false)?;
    require(parts.next().is_none())?;
    Ok(Some(cell))
}
fn width(text: &str) -> usize {
    let mut chars = text.chars();
    if let (Some(base), Some(mark), None) = (chars.next(), chars.next(), chars.next())
        && ('\u{ff66}'..='\u{ff9d}').contains(&base)
        && matches!(mark, '\u{ff9e}' | '\u{ff9f}')
    {
        return 2;
    }
    UnicodeWidthStr::width(text)
}
struct Display {
    columns: u16,
    rows: u16,
    seq: u64,
    cells: Vec<GridCell>,
    cursor: GridCursor,
}
impl Display {
    fn new(columns: u16, rows: u16) -> Result<Self> {
        geometry(columns, rows)?;
        Ok(Self {
            columns,
            rows,
            seq: 0,
            cells: vec![blank(); usize::from(columns) * usize::from(rows)],
            cursor: GridCursor {
                column: Uint(0),
                row: Uint(0),
                visible: false,
                shape: Uint(0),
            },
        })
    }
    fn clear_cell(&mut self, index: usize) {
        let columns = usize::from(self.columns);
        if self.cells[index].width.0 == 0 && index % columns > 0 {
            self.cells[index - 1] = blank();
        }
        if self.cells[index].width.0 == 2 && index % columns + 1 < columns {
            self.cells[index + 1] = blank();
        }
    }
    fn apply(&mut self, seq: u64, columns: u16, rows: u16, full: bool, bytes: &[u8]) -> Result<()> {
        require(
            columns == self.columns
                && rows == self.rows
                && self.seq.checked_add(1) == Some(seq)
                && seq <= UINT_MAX
                && (self.seq != 0 || full),
        )?;
        let text = std::str::from_utf8(bytes).map_err(|_| ErrorCode::NotReady)?;
        let text = text.strip_prefix(PREFIX).ok_or(ErrorCode::NotReady)?;
        let (mut body, suffix) = text.split_once(END).ok_or(ErrorCode::NotReady)?;
        if full {
            self.cells.fill(blank());
        }
        let mut pen = None;
        let mut ink: Option<GridCell> = None;
        let mut linked = false;
        let mut final_cursor = false;
        let mut shape = false;
        let mut clear = false;
        let mut operations = 0;
        while !body.is_empty() {
            operations += 1;
            require(operations <= MAX_CELLS * 16 && !final_cursor)?;
            if let Some(rest) = body.strip_prefix("\x1b[") {
                let end = rest
                    .bytes()
                    .position(|b| (0x40..=0x7e).contains(&b))
                    .ok_or(ErrorCode::NotReady)?;
                require(end <= 128)?;
                let parameters = &rest[..end];
                let command = rest.as_bytes()[end];
                body = &rest[end + 1..];
                match command {
                    b'H' => {
                        require(!shape)?;
                        pen = Some(position(parameters, columns, rows)?);
                    }
                    b'm' => {
                        require(!shape)?;
                        ink = style(parameters)?;
                    }
                    b'J' => {
                        require(
                            parameters == "2"
                                && full
                                && self.seq == 0
                                && !clear
                                && pen.is_none()
                                && ink.is_none(),
                        )?;
                        clear = true;
                    }
                    b'q' => {
                        let p = parameters.strip_suffix(' ').ok_or(ErrorCode::NotReady)?;
                        require(ink.is_none() && !linked && !shape && pen.is_some())?;
                        self.cursor.shape = Uint(number(p, 6)? as u64);
                        shape = true;
                    }
                    b'h' | b'l' => {
                        require(parameters == "?25" && ink.is_none() && !linked)?;
                        let (x, y) = pen.ok_or(ErrorCode::NotReady)?;
                        require(x < usize::from(columns))?;
                        self.cursor.column = Uint(x as u64);
                        self.cursor.row = Uint(y as u64);
                        self.cursor.visible = command == b'h';
                        final_cursor = true;
                    }
                    _ => return Err(ErrorCode::NotReady),
                }
            } else if let Some(rest) = body.strip_prefix("\x1b]8;;") {
                require(!shape)?;
                let end = rest.find("\x1b\\").ok_or(ErrorCode::NotReady)?;
                let uri = &rest[..end];
                require(!uri.chars().any(char::is_control))?;
                linked = !uri.is_empty();
                body = &rest[end + 2..];
            } else {
                require(!shape)?;
                let end = body.find('\x1b').unwrap_or(body.len());
                require(end > 0)?;
                let text = &body[..end];
                body = &body[end..];
                let template = ink.as_ref().ok_or(ErrorCode::NotReady)?;
                for symbol in text.graphemes(true) {
                    operations += 1;
                    require(
                        operations <= MAX_CELLS * 16
                            && symbol.len() <= 260
                            && symbol.chars().take(66).count() <= 65
                            && !symbol
                                .chars()
                                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
                    )?;
                    let w = width(symbol);
                    require((1..=2).contains(&w))?;
                    let (x, y) = pen.ok_or(ErrorCode::NotReady)?;
                    require(x < usize::from(columns))?;
                    let index = y * usize::from(columns) + x;
                    self.clear_cell(index);
                    let visible_width = w.min(usize::from(columns) - x);
                    if visible_width == 2 {
                        self.clear_cell(index + 1);
                    }
                    let cell = GridCell {
                        text: symbol.to_owned(),
                        width: Uint(visible_width as u64),
                        foreground: template.foreground.clone(),
                        background: template.background.clone(),
                        attributes: template.attributes.clone(),
                    };
                    cell.validate().map_err(|_| ErrorCode::NotReady)?;
                    if visible_width == 2 {
                        self.cells[index + 1] = GridCell {
                            text: String::new(),
                            width: Uint(0),
                            foreground: cell.foreground.clone(),
                            background: cell.background.clone(),
                            attributes: cell.attributes.clone(),
                        };
                    }
                    self.cells[index] = cell;
                    pen = Some((x + w, y));
                }
            }
        }
        require(final_cursor && !linked)?;
        // Linux repeats the resolved cursor after sync-end. Consume and compare it,
        // never publish at the marker while leaving unvalidated native bytes behind.
        if !suffix.is_empty() {
            let rest = suffix.strip_prefix("\x1b[").ok_or(ErrorCode::NotReady)?;
            let (p, visibility) = rest.split_once('H').ok_or(ErrorCode::NotReady)?;
            let (x, y) = position(p, columns, rows)?;
            require(
                x as u64 == self.cursor.column.0
                    && y as u64 == self.cursor.row.0
                    && visibility
                        == if self.cursor.visible {
                            "\x1b[?25h"
                        } else {
                            "\x1b[?25l"
                        },
            )?;
        }
        self.seq = seq;
        Ok(())
    }
    fn projection(&self) -> Result<endpoint::Projection> {
        let begin = GridBegin {
            mode: GridMode::Replace,
            revision: Uint(self.seq - 1),
            columns: Uint(u64::from(self.columns)),
            rows: Uint(u64::from(self.rows)),
            cursor: Some(self.cursor.clone()),
            alternate_screen: false,
            graphics_omitted: true,
            popup_blocked: false,
            part_count: Uint(self.cells.len().div_ceil(512) as u64),
            scroll: None,
        };
        begin.validate().map_err(|_| ErrorCode::NotReady)?;
        Ok(endpoint::Projection {
            begin,
            cells: self.cells.clone(),
        })
    }
}
/// A dedicated direct-reader instance retains partial prefixes under select cancellation.
/// Its only writes are the two finite native messages below; it owns no command lane.
pub(super) struct Session {
    pub client: Client,
    reader: endpoint::Reader,
    original: Source,
    lease: std::sync::Weak<grid::GridLease>,
    auth: Arc<Authorized>,
    display: Display,
}
impl Session {
    pub(super) async fn connect(
        context: &grid::AgentObserveContext,
        lease: std::sync::Weak<grid::GridLease>,
        auth: Arc<Authorized>,
    ) -> Result<Self> {
        context.source.recheck().map_err(policy)?;
        let parent = context
            .source
            .socket_path
            .parent()
            .ok_or(ErrorCode::NotReady)?;
        let stem = context
            .source
            .socket_path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or(ErrorCode::NotReady)?;
        let client = Client::connect(&parent.join(format!("{stem}-client.sock")))
            .await
            .map_err(policy)?
            .retain_executable(context.source.executable.clone())
            .map_err(policy)?;
        require(
            client.evidence.linux_boot_id == context.source.evidence.linux_boot_id
                && client.evidence.pid == context.source.evidence.pid
                && client.evidence.pid_start_ticks == context.source.evidence.pid_start_ticks,
        )?;
        context.source.recheck().map_err(policy)?;
        Ok(Self {
            client,
            reader: endpoint::Reader::observer(),
            original: context.source.clone(),
            lease,
            auth,
            display: Display::new(context.columns, context.rows)?,
        })
    }
    fn recheck(&self) -> Result<()> {
        self.auth.live_connection()?;
        self.original.recheck_identity().map_err(policy)?;
        self.client.recheck().map_err(policy)?;
        self.lease
            .upgrade()
            .ok_or(ErrorCode::StaleSession)?
            .check_current()
    }
    async fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.recheck()?;
        let lease = self.lease.upgrade().ok_or(ErrorCode::StaleSession)?;
        let auth = &self.auth;
        let original = &self.original;
        let mut write = std::pin::pin!(self.client.reader.get_mut().write_all(bytes));
        std::future::poll_fn(|cx| {
            if let Err(code) = auth
                .live_connection()
                .and_then(|()| lease.check_current())
                .and_then(|()| original.recheck_identity().map_err(policy))
            {
                return std::task::Poll::Ready(Err(code));
            }
            write.as_mut().poll(cx).map(|result| result.map_err(policy))
        })
        .await
    }
    pub(super) async fn negotiate(&mut self, terminal: &Id) -> Result<()> {
        self.send(&hello(self.display.columns, self.display.rows)?)
            .await?;
        loop {
            let bytes = self.reader.read(&mut self.client.reader).await?;
            self.recheck()?;
            match endpoint::observer_decode(&bytes)? {
                endpoint::ObserverMessage::Welcome {
                    version,
                    encoding,
                    error,
                } => {
                    require(version == 22 && encoding == 1 && !error)?;
                    break;
                }
                endpoint::ObserverMessage::Auxiliary => {}
                _ => return Err(ErrorCode::NotReady),
            }
        }
        self.send(&observe(terminal)?).await
    }
    pub(super) fn deadline(&self) -> Instant {
        self.reader.deadline.expect("complete observer message")
    }
    pub(super) async fn next(&mut self) -> Result<endpoint::Projection> {
        loop {
            let bytes = self.reader.read(&mut self.client.reader).await?;
            self.recheck()?;
            match endpoint::observer_decode(&bytes)? {
                endpoint::ObserverMessage::Frame {
                    seq,
                    width,
                    height,
                    full,
                    bytes,
                } => {
                    self.display.apply(seq, width, height, full, bytes)?;
                    return self.display.projection();
                }
                endpoint::ObserverMessage::Auxiliary => {}
                endpoint::ObserverMessage::Shutdown => return Err(ErrorCode::StaleSession),
                endpoint::ObserverMessage::Welcome { .. } => return Err(ErrorCode::NotReady),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(body: &str, cursor: &str) -> Vec<u8> {
        format!("{PREFIX}{body}{cursor}{END}{cursor}").into_bytes()
    }
    const CURSOR: &str = "\x1b[1;3H\x1b[?25l";
    const DEFAULT: &str = "\x1b[0;39;49m";
    #[test]
    fn installed_wire_vectors_and_auxiliary_validation() {
        assert_eq!(hello(3, 2).unwrap(), [7, 0, 0, 0, 0, 22, 3, 2, 0, 0, 0]);
        assert_eq!(
            observe(&Id("term-x".into())).unwrap(),
            [8, 0, 0, 0, 7, 6, b't', b'e', b'r', b'm', b'-', b'x']
        );
        assert!(matches!(
            endpoint::observer_decode(&[0, 22, 1, 0]).unwrap(),
            endpoint::ObserverMessage::Welcome {
                version: 22,
                encoding: 1,
                error: false
            }
        ));
        assert!(matches!(
            endpoint::observer_decode(&[7]).unwrap(),
            endpoint::ObserverMessage::Auxiliary
        ));
        assert!(endpoint::observer_decode(&[7, 0]).is_err());
        assert!(matches!(
            endpoint::observer_decode(&[3, 0]).unwrap(),
            endpoint::ObserverMessage::Shutdown
        ));
        let bytes = [1, 1, 3, 2, 1, 1, b'X'];
        match endpoint::observer_decode(&bytes).unwrap() {
            endpoint::ObserverMessage::Frame {
                seq,
                width,
                height,
                full,
                bytes,
            } => assert_eq!(
                (seq, width, height, full, bytes),
                (1, 3, 2, true, &b"X"[..])
            ),
            _ => panic!(),
        }
        for bad in [
            &[1, 1, 3, 2, 2, 0][..],
            &[1, 1, 3, 2, 1, 2, b'X'][..],
            &[0, 22, 2, 0][..],
            &[3, 2][..],
        ] {
            assert!(endpoint::observer_decode(bad).is_err());
        }
    }
    #[test]
    fn source_full_partial_cursor_only_linux_suffix_and_full_without_clear() {
        let mut display = Display::new(3, 2).unwrap();
        let cursor = "\x1b[1;2H\x1b[2 q\x1b[?25h";
        let first = format!(
            "{PREFIX}\x1b[2J\x1b[1;1H{DEFAULT}ABC\x1b[2;1HDEF\x1b[0m{cursor}{END}\x1b[1;2H\x1b[?25h"
        );
        display.apply(1, 3, 2, true, first.as_bytes()).unwrap();
        assert_eq!(
            display
                .cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>(),
            "ABCDEF"
        );
        assert_eq!(display.cursor.shape, Uint(2));
        assert!(display.cursor.visible);
        let partial = frame(&format!("\x1b[1;2H{DEFAULT}x\x1b[0m"), "\x1b[1;2H\x1b[?25h");
        display.apply(2, 3, 2, false, &partial).unwrap();
        assert_eq!(display.cells[1].text, "x");
        let cursor_only = format!("{PREFIX}\x1b[2;3H\x1b[6 q\x1b[?25l{END}\x1b[2;3H\x1b[?25l");
        display
            .apply(3, 3, 2, false, cursor_only.as_bytes())
            .unwrap();
        assert_eq!(display.cells[1].text, "x");
        assert_eq!(display.cursor.shape, Uint(6));
        display
            .apply(
                4,
                3,
                2,
                true,
                &frame(&format!("\x1b[1;1H{DEFAULT}Q\x1b[0m"), "\x1b[1;1H\x1b[?25l"),
            )
            .unwrap();
        assert_eq!(display.cells[0].text, "Q");
        assert_eq!(display.cells[1].text, " ");
        let projection = display.projection().unwrap();
        assert!(projection.begin.graphics_omitted);
        assert!(!projection.begin.alternate_screen);
        assert_eq!(projection.begin.revision, Uint(3));
    }
    #[test]
    fn source_styles_links_graphemes_wide_tail_and_right_crop() {
        let mut display = Display::new(3, 1).unwrap();
        let body = "\x1b[1;1H\x1b[0;1;2;3;4:3;5;6;7;8;9;38;5;196;48;2;1;2;3mX\x1b]8;;https://example.com/a;b\x1b\\Y\x1b]8;;\x1b\\\x1b[0;39;49mZ\x1b[0m";
        display.apply(1, 3, 1, true, &frame(body, CURSOR)).unwrap();
        assert!(
            display.cells[0].attributes.bold
                && display.cells[0].attributes.faint
                && display.cells[0].attributes.italic
                && display.cells[0].attributes.blink
                && display.cells[0].attributes.strike
                && display.cells[0].attributes.inverse
                && display.cells[0].attributes.concealed
        );
        assert_eq!(
            display.cells[0].foreground,
            GridColor::Indexed { index: Uint(196) }
        );
        assert_eq!(
            display.cells[0].background,
            GridColor::Rgb {
                value: Uint(0x010203)
            }
        );
        assert_eq!(display.cells[0].attributes.underline, GridUnderline::Curly);
        assert_eq!(display.cells[2].attributes, attributes());
        let mut seq = 1;
        for glyph in ["界", "ｶﾞ", "🇧🇷", "👨‍👩‍👧", "⚠️"] {
            seq += 1;
            display
                .apply(
                    seq,
                    3,
                    1,
                    false,
                    &frame(
                        &format!("\x1b[1;1H{DEFAULT}{glyph}\x1b[1;3HZ\x1b[0m"),
                        CURSOR,
                    ),
                )
                .unwrap();
            assert_eq!(display.cells[0].text, glyph);
            assert_eq!(display.cells[0].width, Uint(2));
            assert_eq!(display.cells[1].width, Uint(0));
            seq += 1;
            display
                .apply(
                    seq,
                    3,
                    1,
                    false,
                    &frame(&format!("\x1b[1;1H{DEFAULT}A\x1b[1;2H \x1b[0m"), CURSOR),
                )
                .unwrap();
            assert_eq!(display.cells[1].text, " ");
        }
        let long = format!("e{}", "\u{301}".repeat(64));
        seq += 1;
        display
            .apply(
                seq,
                3,
                1,
                false,
                &frame(
                    &format!("\x1b[1;1H{DEFAULT}{long}\x1b[1;2HQ \x1b[0m"),
                    CURSOR,
                ),
            )
            .unwrap();
        assert_eq!(display.cells[0].text, long);
        seq += 1;
        display
            .apply(
                seq,
                3,
                1,
                false,
                &frame(&format!("\x1b[1;3H{DEFAULT}界\x1b[0m"), CURSOR),
            )
            .unwrap();
        assert_eq!(display.cells[2].text, "界");
        assert_eq!(display.cells[2].width, Uint(1));
        for underline in ["4", "4:2", "4:3", "4:4", "4:5"] {
            assert!(
                style(&format!("0;{underline};38;5;255;48;2;0;255;0"))
                    .unwrap()
                    .is_some()
            );
        }
        for fg in ["30", "37", "90", "97", "38;5;0", "38;2;255;0;255"] {
            assert!(style(&format!("0;{fg};49")).is_ok());
        }
    }
    #[test]
    fn malformed_sequence_control_escape_utf8_and_work_limits_fail_closed() {
        for body in [
            "\x1b_Ga=T,f=32,t=d,i=7,p=1,s=1,v=1,q=2;/wAA/w==\x1b\\",
            "\r",
            "\n",
            "\t",
            "\x1b]52;;x\x1b\\",
            "\x1b[99999;1H",
            "\x1b[0;1H",
            "\x1b[1H",
            "\x1b[01;1H",
            "\x1b[1;1H\x1b[0;38;5;256;49mX",
            "\x1b[1;1H\x1b[0;4:1;39;49mX",
            "\x1b[1;1H\x1b[0;39;49mXXXX",
            "\x1b]8;;bad\x07\x1b\\",
            "\x1b[1;1H\x1b[0;39;49m\u{9c}",
        ] {
            assert!(
                Display::new(3, 1)
                    .unwrap()
                    .apply(1, 3, 1, true, &frame(body, CURSOR))
                    .is_err()
            );
        }
        let valid = frame(&format!("\x1b[1;1H{DEFAULT}Ü  \x1b[0m"), CURSOR);
        let mut display = Display::new(3, 1).unwrap();
        assert!(display.apply(1, 3, 1, false, &valid).is_err());
        display.apply(1, 3, 1, true, &valid).unwrap();
        assert!(display.apply(1, 3, 1, true, &valid).is_err());
        assert!(display.apply(3, 3, 1, true, &valid).is_err());
        assert!(display.apply(2, 2, 1, true, &valid).is_err());
        let mut suffix = valid.clone();
        suffix.push(b'X');
        assert!(display.apply(2, 3, 1, false, &suffix).is_err());
        let long = format!("\x1b[1;1H{DEFAULT}e{}\x1b[0m", "\u{301}".repeat(65));
        assert!(
            Display::new(3, 1)
                .unwrap()
                .apply(1, 3, 1, true, &frame(&long, CURSOR))
                .is_err()
        );
        let repeats = "\x1b[1;1H".repeat(MAX_CELLS * 16 + 1);
        assert!(
            Display::new(3, 1)
                .unwrap()
                .apply(1, 3, 1, true, &frame(&repeats, CURSOR))
                .is_err()
        );
        assert!(Display::new(512, 17).is_err());
        assert_eq!(viewport(512, 256), (512, 16));
        let mut invalid = valid;
        invalid.insert(PREFIX.len(), 0xff);
        assert!(
            Display::new(3, 1)
                .unwrap()
                .apply(1, 3, 1, true, &invalid)
                .is_err()
        );
    }
    #[tokio::test]
    async fn fragmented_reader_and_distinct_direct_frame_limit() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut reader = BufReader::new(reader);
        let mut direct = endpoint::Reader::observer();
        let send = tokio::spawn(async move {
            for byte in [4, 0, 0, 0, 0, 22, 1, 0] {
                writer.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert_eq!(direct.read(&mut reader).await.unwrap(), [0, 22, 1, 0]);
        send.await.unwrap();
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut reader = BufReader::new(reader);
        let mut partial = endpoint::Reader::observer();
        writer.write_all(&[4, 0]).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(1), partial.read(&mut reader))
                .await
                .is_err()
        );
        writer.write_all(&[0, 0, 0, 22, 1, 0]).await.unwrap();
        assert_eq!(partial.read(&mut reader).await.unwrap(), [0, 22, 1, 0]);
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut reader = BufReader::new(reader);
        let mut ordinary = endpoint::Reader::new();
        writer
            .write_all(&((endpoint::MAX_FRAME + 1) as u32).to_le_bytes())
            .await
            .unwrap();
        assert!(ordinary.read(&mut reader).await.is_err());
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut reader = BufReader::new(reader);
        writer
            .write_all(&(32u32 * 1024 * 1024 + 1).to_le_bytes())
            .await
            .unwrap();
        assert!(
            endpoint::Reader::observer()
                .read(&mut reader)
                .await
                .is_err()
        );
    }
    #[test]
    fn maximum_stock_graphemes_fit_the_existing_multipart_publication_quota() {
        use std::fmt::Write;
        let glyph = format!("\u{1d11e}{}", "\u{1d165}".repeat(64));
        assert_eq!(glyph.len(), 260);
        assert_eq!(glyph.chars().count(), 65);
        let mut body = String::with_capacity(MAX_CELLS * 280);
        body.push_str("\x1b[2J");
        for y in 1..=16 {
            for x in 1..=512 {
                write!(body, "\x1b[{y};{x}H").unwrap();
                if x == 1 && y == 1 {
                    body.push_str(DEFAULT);
                }
                body.push_str(&glyph);
            }
        }
        body.push_str("\x1b[0m");
        let mut display = Display::new(512, 16).unwrap();
        display
            .apply(1, 512, 16, true, &frame(&body, "\x1b[16;512H\x1b[?25l"))
            .unwrap();
        let projection = display.projection().unwrap();
        assert_eq!(projection.begin.part_count, Uint(16));
        let subscription = Uuid::random();
        let mut total = 0;
        for (index, cells) in projection.cells.chunks(512).enumerate() {
            let event = Envelope::Event(Event {
                subscription: subscription.clone(),
                seq: Uint(index as u64 + 1),
                body: EventBody::GridPart(GridPart {
                    revision: Uint(0),
                    index: Uint(index as u64),
                    spans: vec![GridSpan {
                        row: Uint(index as u64),
                        column: Uint(0),
                        cells: cells.to_vec(),
                    }],
                }),
            });
            let bytes = crate::protocol::encode_payload(&event).unwrap();
            assert!(bytes.len() <= FRAME_MAX);
            total += bytes.len();
        }
        assert!(total + 4096 <= 8 * 1024 * 1024);
    }
    #[tokio::test]
    async fn direct_reader_accepts_stock_auxiliary_larger_than_the_shell_cap() {
        let length = endpoint::MAX_FRAME + 1;
        let mut payload = vec![2];
        uint(&mut payload, length as u64);
        payload.resize(payload.len() + length, 0);
        let mut bytes = vec![0; 4];
        bytes.extend_from_slice(&payload);
        let bytes = framed(bytes);
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut reader = BufReader::new(reader);
        let sending = tokio::spawn(async move {
            writer.write_all(&bytes).await.unwrap();
        });
        let payload = endpoint::Reader::observer()
            .read(&mut reader)
            .await
            .unwrap();
        assert!(matches!(
            endpoint::observer_decode(&payload).unwrap(),
            endpoint::ObserverMessage::Auxiliary
        ));
        sending.await.unwrap();
    }
}
