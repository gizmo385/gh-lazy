//! Source to block tree, over `pulldown-cmark`'s events, reading the document the way markdown-it's `gfm-like` preset
//! and Textual's `Markdown` widget do in Python.

use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, OffsetIter, Options, Parser, Tag, TagEnd};

use super::normalize::{normalize_link, normalize_link_text, validate_link};
use super::{Block, Inline, InlineStyle, linkify};

/// How deep quotes and lists nest before the rest of a container reads as one paragraph of its text. Layout recurses
/// per level, and no README nests this far.
const MAX_DEPTH: usize = 32;

/// What Textual shows for an image, before its alt text.
const IMAGE_PREFIX: &str = "🖼  ";

pub(super) fn parse(source: &str) -> Vec<Block> {
    // Tables and strikethrough only, as in `gfm-like`: no task lists, footnotes, smart punctuation or heading
    // attributes. Linkify isn't a pulldown-cmark option: `Inlines` splits each text token at its links.
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut parser = BlockParser {
        events: Parser::new_ext(source, options).into_offset_iter(),
        source,
        depth: 0,
    };
    parser.blocks(None)
}

struct BlockParser<'a> {
    events: OffsetIter<'a>,
    source: &'a str,
    /// How many quotes and list items enclose the blocks being read.
    depth: usize,
}

impl<'a> BlockParser<'a> {
    /// Reads blocks up to the `end` of their container, or to the end of the input.
    fn blocks(&mut self, end: Option<TagEnd>) -> Vec<Block> {
        let mut blocks = Vec::new();
        // Inline content outside a paragraph: a tight list item's text, which markdown-it puts in a (hidden)
        // paragraph and pulldown-cmark doesn't.
        let mut loose = Inlines::new(self.source);
        let flush = |loose: &mut Inlines, blocks: &mut Vec<Block>| {
            if !loose.is_empty() {
                let source = loose.source;
                blocks.push(Block::Paragraph(
                    std::mem::replace(loose, Inlines::new(source)).finish(),
                ));
            }
        };
        while let Some((event, range)) = self.events.next() {
            match event {
                Event::End(tag) if Some(tag) == end => break,
                Event::Start(tag) if is_inline(&tag) => loose.start(&tag, range),
                Event::End(tag) if is_inline_end(tag) => loose.end(tag, &range),
                Event::Start(tag) => {
                    flush(&mut loose, &mut blocks);
                    if let Some(block) = self.block(tag) {
                        blocks.push(block);
                    }
                }
                Event::Rule => {
                    flush(&mut loose, &mut blocks);
                    blocks.push(Block::Rule);
                }
                // HTML blocks vanish, as Textual drops markdown-it's `html_block` tokens. Any other stray end can't
                // come from the options enabled.
                Event::Html(_) | Event::End(_) => {}
                event => loose.event(event, &range),
            }
        }
        flush(&mut loose, &mut blocks);
        blocks
    }

    /// The block a non-inline tag starts, read through its end. `None` for what isn't shown.
    fn block(&mut self, tag: Tag<'a>) -> Option<Block> {
        Some(match tag {
            Tag::Paragraph => Block::Paragraph(self.inlines(TagEnd::Paragraph)),
            Tag::Heading { level, .. } => Block::Heading {
                level: heading_level(level),
                text: self.inlines(TagEnd::Heading(level)),
            },
            Tag::BlockQuote(kind) => {
                let end = TagEnd::BlockQuote(kind);
                Block::Quote(self.nested(end))
            }
            Tag::List(start) => self.list(start),
            Tag::CodeBlock(kind) => self.code(kind),
            Tag::Table(_) => self.table(),
            other => {
                // An HTML block, or an item outside a list, which pulldown-cmark doesn't emit: skip it whole.
                self.skip(other.to_end());
                return None;
            }
        })
    }

    /// A container's blocks. Past [`MAX_DEPTH`], its text as one paragraph.
    fn nested(&mut self, end: TagEnd) -> Vec<Block> {
        if self.depth >= MAX_DEPTH {
            return vec![Block::Paragraph(self.flatten(end))];
        }
        self.depth += 1;
        let blocks = self.blocks(Some(end));
        self.depth -= 1;
        blocks
    }

    fn list(&mut self, start: Option<u64>) -> Block {
        let end = TagEnd::List(start.is_some());
        let mut items = Vec::new();
        while let Some((event, _)) = self.events.next() {
            match event {
                Event::Start(Tag::Item) => items.push(self.nested(TagEnd::Item)),
                Event::End(tag) if tag == end => break,
                // A list holds only items.
                _ => {}
            }
        }
        Block::List { start, items }
    }

    fn code(&mut self, kind: CodeBlockKind<'a>) -> Block {
        let language = match kind {
            CodeBlockKind::Fenced(info) if !info.trim().is_empty() => Some(info.trim().to_owned()),
            _ => None,
        };
        let mut text = String::new();
        for (event, _) in self.events.by_ref() {
            match event {
                Event::Text(chunk) => text.push_str(&chunk),
                Event::End(TagEnd::CodeBlock) => break,
                _ => {}
            }
        }
        Block::Code { language, text }
    }

    fn table(&mut self) -> Block {
        let mut head = Vec::new();
        let mut rows = Vec::new();
        let mut row = Vec::new();
        while let Some((event, _)) = self.events.next() {
            match event {
                Event::Start(Tag::TableCell) => row.push(self.inlines(TagEnd::TableCell)),
                Event::End(TagEnd::TableHead) => head = std::mem::take(&mut row),
                Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        // As GFM: every row has the header's cells, short rows padded with empty ones and extra cells dropped.
        for row in &mut rows {
            row.resize_with(head.len(), Vec::new);
        }
        Block::Table { head, rows }
    }

    /// Inline content up to `end`.
    fn inlines(&mut self, end: TagEnd) -> Vec<Inline> {
        let mut inlines = Inlines::new(self.source);
        for (event, range) in self.events.by_ref() {
            match event {
                Event::End(tag) if tag == end => break,
                Event::Start(tag) => inlines.start(&tag, range),
                Event::End(tag) => inlines.end(tag, &range),
                event => inlines.event(event, &range),
            }
        }
        inlines.finish()
    }

    /// The plain text of everything up to `end`, as one run.
    fn flatten(&mut self, end: TagEnd) -> Vec<Inline> {
        let mut inlines = Inlines::new(self.source);
        let mut open = 0_usize;
        for (event, _) in self.events.by_ref() {
            match event {
                Event::End(tag) if open == 0 && tag == end => break,
                Event::Start(_) => open += 1,
                Event::End(_) => open = open.saturating_sub(1),
                Event::Text(text) | Event::Code(text) => inlines.text(&text, true),
                Event::SoftBreak | Event::HardBreak => inlines.text(" ", true),
                _ => {}
            }
        }
        inlines.finish()
    }

    /// Skips everything up to `end`.
    fn skip(&mut self, end: TagEnd) {
        let mut open = 0_usize;
        for (event, _) in self.events.by_ref() {
            match event {
                Event::End(tag) if open == 0 && tag == end => break,
                Event::Start(_) => open += 1,
                Event::End(_) => open = open.saturating_sub(1),
                _ => {}
            }
        }
    }
}

fn is_inline(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }
    )
}

fn is_inline_end(tag: TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image
    )
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Builds the inline runs of one block, as Textual's `_token_to_content` turns markdown-it's inline tokens into
/// styled text.
#[derive(Debug)]
struct Inlines<'a> {
    /// The document, which the events' ranges index.
    source: &'a str,
    runs: Vec<Inline>,
    strong: usize,
    emphasis: usize,
    /// The open strikethroughs. pulldown-cmark also strikes `~text~`, which markdown-it leaves as text, so each
    /// records whether it's a real `~~` one, and whether it opened outside an image and so styles its text.
    strikes: Vec<Strike>,
    links: Vec<Arc<str>>,
    /// How many images enclose the current text. An image's alt text is plain: Textual adds its tokens' text without
    /// their styles.
    images: usize,
    /// Whether the next text event continues the last one, as one markdown-it text token. Anything in between, even
    /// dropped inline HTML, makes it a token of its own, whose whitespace collapses separately.
    joins: bool,
    /// The current text token's text, outside links and images, held until the token ends so linkify can look for
    /// bare URLs in all of it, as markdown-it's linkify does per text token.
    pending: String,
    /// The links and images open, innermost last.
    opened: Vec<Opened>,
    /// Whether the text events of the autolink being read are skipped: its text is already in, normalized.
    autolink: bool,
}

#[derive(Debug, Clone, Copy)]
struct Strike {
    real: bool,
    styled: bool,
}

/// A link or image being read.
#[derive(Debug, Clone, Copy)]
enum Opened {
    Tag,
    /// One whose target fails `validate_link`, which markdown-it doesn't make: its markup shows as text around its
    /// (still parsed) link text. `end` is where its source ends, `inner_end` where the last event inside it ended.
    /// The markup after the link text shows as written, where markdown-it parses it again, so a backslash escape or
    /// an entity in a rejected target shows unexpanded.
    Literal {
        end: usize,
        inner_end: usize,
    },
}

impl<'a> Inlines<'a> {
    fn new(source: &'a str) -> Self {
        Inlines {
            source,
            runs: Vec::new(),
            strong: 0,
            emphasis: 0,
            strikes: Vec::new(),
            links: Vec::new(),
            images: 0,
            joins: false,
            pending: String::new(),
            opened: Vec::new(),
            autolink: false,
        }
    }

    /// Records that an event inside the open literal links ended at `range.end`.
    fn note(&mut self, range: &Range<usize>) {
        for opened in &mut self.opened {
            if let Opened::Literal { inner_end, .. } = opened {
                *inner_end = (*inner_end).max(range.end);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.runs.is_empty() && self.pending.is_empty()
    }

    fn finish(mut self) -> Vec<Inline> {
        self.flush();
        self.runs
    }

    fn style(&self) -> InlineStyle {
        InlineStyle {
            strong: self.strong > 0,
            emphasis: self.emphasis > 0,
            strike: self.strikes.iter().any(|s| s.real && s.styled),
            code: false,
        }
    }

    fn start(&mut self, tag: &Tag<'_>, range: Range<usize>) {
        self.note(&range);
        let source = &self.source[range.clone()];
        let link = match tag {
            Tag::Link {
                link_type, dest_url, ..
            } => Some(LinkTarget::new(*link_type, dest_url, source)),
            Tag::Image { dest_url, .. } => Some(LinkTarget {
                url: normalize_link(dest_url),
                text: None,
            }),
            _ => None,
        };
        if let Some(link) = &link
            && !validate_link(&link.url)
        {
            // markdown-it's link rule fails, so the markup is text in the same text token as what's around it.
            let markup = match tag {
                Tag::Image { .. } => "![",
                Tag::Link {
                    link_type: LinkType::Autolink | LinkType::Email,
                    ..
                } => "<",
                _ => "[",
            };
            self.opened.push(Opened::Literal {
                end: range.end,
                inner_end: range.start + markup.len(),
            });
            self.text(markup, self.images == 0);
            return;
        }
        if link.is_some() {
            self.opened.push(Opened::Tag);
        }
        self.flush();
        self.joins = false;
        match (tag, link) {
            (Tag::Image { .. }, Some(image)) => {
                if self.images == 0 {
                    self.links.push(Arc::from(image.url));
                    self.text(IMAGE_PREFIX, false);
                }
                self.images += 1;
            }
            (Tag::Link { .. }, Some(link)) => {
                // Inside an image, only the text counts.
                if self.images == 0 {
                    self.links.push(Arc::from(link.url));
                }
                if let Some(text) = link.text {
                    self.text(&text, self.images == 0);
                    self.autolink = true;
                }
            }
            (tag, _) => self.start_style(tag, source),
        }
    }

    fn start_style(&mut self, tag: &Tag<'_>, source: &str) {
        match tag {
            Tag::Strikethrough => {
                let strike = Strike {
                    real: source.starts_with("~~"),
                    styled: self.images == 0,
                };
                if !strike.real {
                    self.text("~", false);
                }
                self.strikes.push(strike);
            }
            // Inside an image, only the text counts.
            _ if self.images > 0 => {}
            Tag::Strong => self.strong += 1,
            Tag::Emphasis => self.emphasis += 1,
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd, range: &Range<usize>) {
        if matches!(tag, TagEnd::Link | TagEnd::Image) {
            self.autolink = false;
            if let Some(Opened::Literal { end, inner_end }) = self.opened.pop() {
                let markup = self.source.get(inner_end.min(end)..end).unwrap_or_default();
                self.note(range);
                self.text(markup, self.images == 0);
                return;
            }
        }
        self.note(range);
        self.flush();
        self.joins = false;
        match tag {
            TagEnd::Strikethrough => {
                if let Some(strike) = self.strikes.pop()
                    && !strike.real
                {
                    self.text("~", false);
                }
            }
            TagEnd::Image => {
                self.images = self.images.saturating_sub(1);
                if self.images == 0 {
                    self.links.pop();
                }
            }
            _ if self.images > 0 => {}
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event<'_>, range: &Range<usize>) {
        self.note(range);
        if self.autolink && matches!(event, Event::Text(_)) {
            return;
        }
        let in_image = self.images > 0;
        if !matches!(event, Event::Text(_)) {
            self.flush();
            self.joins = false;
        }
        match event {
            Event::Text(text) => self.text(&text, !in_image),
            Event::Code(code) if in_image => self.text(&code, false),
            Event::Code(code) => self.push(
                code.as_ref(),
                InlineStyle {
                    code: true,
                    ..self.style()
                },
            ),
            // Inside an image Textual adds each token's `content`, which is empty for a break and the markup for
            // inline HTML.
            Event::InlineHtml(html) if in_image => self.text(&html, false),
            Event::SoftBreak if !in_image => self.text(" ", false),
            Event::HardBreak if !in_image => self.text("\n", false),
            // Inline HTML loses its tags; the text between them comes as text events.
            _ => {}
        }
    }

    /// Adds text in the current style. `collapse` turns each run of whitespace into one space, as Textual does with
    /// a text token (markdown-it joins adjacent text, so a run that continues the previous one collapses with it).
    /// Collapsed text outside links and images waits in [`Inlines::pending`] for linkify.
    fn text(&mut self, text: &str, collapse: bool) {
        if !collapse || !self.joins {
            self.flush();
        }
        if !collapse {
            self.push(text, self.style());
            self.joins = false;
            return;
        }
        let after_space = self.joins
            && if self.pending.is_empty() {
                self.runs.last().is_some_and(|run| run.text.ends_with(is_space))
            } else {
                self.pending.ends_with(is_space)
            };
        let mut collapsed = String::with_capacity(text.len());
        let mut space = after_space;
        for c in text.chars() {
            if is_space(c) {
                if !space {
                    collapsed.push(' ');
                }
                space = true;
            } else {
                collapsed.push(c);
                space = false;
            }
        }
        if self.images == 0 && self.links.is_empty() {
            self.pending.push_str(&collapsed);
        } else {
            self.push(&collapsed, self.style());
        }
        self.joins = true;
    }

    /// Adds the pending text token, its bare URLs, `www.` hosts and emails as links (#218).
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending);
        let style = self.style();
        let mut at = 0;
        for found in linkify::links(&text) {
            self.push_run(&text[at..found.range.start], style, None);
            self.push_run(&found.text, style, Some(Arc::from(found.url)));
            at = found.range.end;
        }
        self.push_run(&text[at..], style, None);
    }

    /// Appends `text` as a run of the link being read, if any.
    fn push(&mut self, text: &str, style: InlineStyle) {
        let link = self.links.last().cloned();
        self.push_run(text, style, link);
    }

    /// Appends `text` as a run, joining it to the last run when they look the same.
    fn push_run(&mut self, text: &str, style: InlineStyle, link: Option<Arc<str>>) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.runs.last_mut()
            && !style.code
            && last.style == style
            && same_link(last.link.as_ref(), link.as_ref())
        {
            last.text.push_str(text);
            return;
        }
        self.runs.push(Inline {
            text: text.to_owned(),
            style,
            link,
        });
    }
}

/// Where a link or image goes, as markdown-it normalizes it.
struct LinkTarget {
    url: String,
    /// An autolink's text, normalized; a link's text is its own.
    text: Option<String>,
}

impl LinkTarget {
    /// `source` is the link's markup. An autolink's text and target come from what's between its `<` and `>` as
    /// written, an email's target with `mailto:`, as markdown-it's `autolink` rule makes them.
    fn new(link_type: LinkType, dest_url: &str, source: &str) -> Self {
        let written = || {
            source
                .strip_prefix('<')
                .and_then(|s| s.strip_suffix('>'))
                .unwrap_or(dest_url)
        };
        match link_type {
            LinkType::Autolink => LinkTarget {
                url: normalize_link(written()),
                text: Some(normalize_link_text(written())),
            },
            LinkType::Email => LinkTarget {
                url: normalize_link(&format!("mailto:{}", written())),
                text: Some(normalize_link_text(written())),
            },
            _ => LinkTarget {
                url: normalize_link(dest_url),
                text: None,
            },
        }
    }
}

/// Whether two runs belong to the same link (or both to none). Two links to the same place are still two links.
fn same_link(a: Option<&Arc<str>>, b: Option<&Arc<str>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// Python's `\s` on `str`: Unicode whitespace, which also counts the four information separators.
pub(super) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
