prpr_l10n::tl_file!("settings");

use super::{NextPage, OffsetPage, Page, SharedState};
use crate::{
    dir, get_data, get_data_mut,
    popup::ChooseButton,
    save_data,
    scene::BGM_VOLUME_UPDATED,
    sync_data,
    tabs::{Tabs, TitleFn},
};
use anyhow::Result;
use bytesize::ByteSize;
use inputbox::InputBox;
use macroquad::prelude::*;
use once_cell::sync::Lazy;
use prpr::{
    config::{
        JudgementMode, JudgementRangeAnchor, JUDGEMENT_RANGE_ALPHA_MAX, JUDGEMENT_RANGE_ALPHA_MIN, JUDGEMENT_RANGE_ALPHA_STEP,
        JUDGEMENT_RANGE_HORIZON_MAX, JUDGEMENT_RANGE_HORIZON_MIN, JUDGEMENT_RANGE_HORIZON_STEP,
    },
    core::BOLD_FONT,
    ext::{open_url, poll_future, semi_white, LocalTask, RectExt, SafeTexture},
    scene::{request_input, return_input, show_error, show_message, take_input},
    task::Task,
    ui::{DRectButton, Scroll, Slider, Ui, PREFER_REDUCED_MOTION, UI_SFX_VOLUME},
};
use prpr_l10n::{LanguageIdentifier, LANG_IDENTS, LANG_NAMES};
use reqwest::Url;
use serde::Deserialize;
use std::{borrow::Cow, fs, io, path::PathBuf, sync::atomic::Ordering};

const ITEM_HEIGHT: f32 = 0.15;
const INTERACT_WIDTH: f32 = 0.26;
const STATUS_PAGE: &str = "https://status.phira.cn";

struct NameList(String);
impl<'de> Deserialize<'de> for NameList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Vec::<String>::deserialize(deserializer)?;
        Ok(Self(s.join(", ")))
    }
}

#[derive(Deserialize)]
struct LocalizationListRaw {
    #[serde(rename = "en-US")]
    en_us: NameList,
    #[serde(rename = "fr-FR")]
    fr_fr: NameList,
    #[serde(rename = "de-DE")]
    de_de: NameList,
    #[serde(rename = "id-ID")]
    id_id: NameList,
    #[serde(rename = "ja-JP")]
    ja_jp: NameList,
    #[serde(rename = "ko-KR")]
    ko_kr: NameList,
    #[serde(rename = "pl-PL")]
    pl_pl: NameList,
    #[serde(rename = "pt-BR")]
    pt_br: NameList,
    #[serde(rename = "ru-RU")]
    ru_ru: NameList,
    #[serde(rename = "th-TH")]
    th_th: NameList,
    #[serde(rename = "zh-TW")]
    zh_tw: NameList,
    #[serde(rename = "tr-TR")]
    tr_tr: NameList,
    #[serde(rename = "vi-VN")]
    vi_vn: NameList,
}

struct LocalizationList(String);
impl<'de> Deserialize<'de> for LocalizationList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = LocalizationListRaw::deserialize(deserializer)?;
        Ok(Self(format!(
            "\
English (en-US)\n{}\n
French (fr-FR)\n{}\n
German (de-DE)\n{}\n
Indonesian (id-ID)\n{}\n
Japanese (ja-JP)\n{}\n
Korean (ko-KR)\n{}\n
Polish (pl-PL)\n{}\n
Portuguese (pt-BR)\n{}\n
Russian (ru-RU)\n{}\n
Thai (th-TH)\n{}\n
Traditional Chinese (zh-TW)\n{}\n
Turkish (tr-TR)\n{}\n
Vietnamese (vi-VN)\n{}",
            raw.en_us.0,
            raw.fr_fr.0,
            raw.de_de.0,
            raw.id_id.0,
            raw.ja_jp.0,
            raw.ko_kr.0,
            raw.pl_pl.0,
            raw.pt_br.0,
            raw.ru_ru.0,
            raw.th_th.0,
            raw.zh_tw.0,
            raw.tr_tr.0,
            raw.vi_vn.0
        )))
    }
}

#[derive(Deserialize)]
struct StaffList {
    development: NameList,
    operations: NameList,
    documentation: NameList,
    art: NameList,
    music: NameList,
    audio: NameList,
    community: NameList,
    localization: LocalizationList,
}

static STAFF_LIST: Lazy<StaffList> = Lazy::new(|| {
    let data = include_str!("../../staff.yml");
    serde_yaml::from_str(data).unwrap()
});

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingListType {
    General,
    Audio,
    Chart,
    Replica,
    Debug,
    About,
}

pub struct SettingsPage {
    list_general: GeneralList,
    list_audio: AudioList,
    list_chart: ChartList,
    list_replica: ReplicaList,
    list_debug: DebugList,

    tabs: Tabs<SettingListType>,

    scroll: Scroll,
    save_time: f32,

    icon: SafeTexture,
}

impl SettingsPage {
    const SAVE_TIME: f32 = 0.5;

    pub fn new(icon: SafeTexture, icon_lang: SafeTexture) -> Self {
        Self {
            list_general: GeneralList::new(icon_lang),
            list_audio: AudioList::new(),
            list_chart: ChartList::new(false),
            list_replica: ReplicaList::new(),
            list_debug: DebugList::new(false),

            tabs: Tabs::new([
                (SettingListType::General, || tl!("general")),
                (SettingListType::Audio, || tl!("audio")),
                (SettingListType::Chart, || tl!("chart")),
                (SettingListType::Replica, || tl!("replica")),
                (SettingListType::Debug, || tl!("debug")),
                (SettingListType::About, || tl!("about")),
            ] as [(SettingListType, TitleFn); 6]),

            scroll: Scroll::new(),
            save_time: f32::INFINITY,

            icon,
        }
    }
}

impl Page for SettingsPage {
    fn label(&self) -> Cow<'static, str> {
        tl!("label")
    }

    fn exit(&mut self) -> Result<()> {
        BGM_VOLUME_UPDATED.store(true, Ordering::Relaxed);
        if self.save_time.is_finite() {
            save_data()?;
        }
        Ok(())
    }

    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        let t = s.t;
        if match self.tabs.selected() {
            SettingListType::General => self.list_general.top_touch(touch, t),
            SettingListType::Audio => self.list_audio.top_touch(touch, t),
            SettingListType::Chart => self.list_chart.top_touch(touch, t),
            SettingListType::Replica => self.list_replica.top_touch(touch, t),
            SettingListType::Debug => self.list_debug.top_touch(touch, t),
            SettingListType::About => false,
        } {
            return Ok(true);
        }

        if self.tabs.touch(touch, s.rt) {
            if self.tabs.changed() {
                self.scroll.y_scroller.reset();
            }
            return Ok(true);
        }

        if self.scroll.touch(touch, t) {
            return Ok(true);
        }
        if let Some(p) = match self.tabs.selected() {
            SettingListType::General => self.list_general.touch(touch, t)?,
            SettingListType::Audio => self.list_audio.touch(touch, t)?,
            SettingListType::Chart => self.list_chart.touch(touch, t)?,
            SettingListType::Replica => self.list_replica.touch(touch, t)?,
            SettingListType::Debug => self.list_debug.touch(touch, t)?,
            SettingListType::About => None,
        } {
            if p {
                self.save_time = t;
            }
            self.scroll.y_scroller.halt();
            return Ok(true);
        }
        Ok(false)
    }

    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        let t = s.t;
        let changed = match self.tabs.selected() {
            SettingListType::General => self.list_general.update(t)?,
            SettingListType::Audio => self.list_audio.update(t)?,
            SettingListType::Chart => self.list_chart.update(t)?,
            SettingListType::Replica => self.list_replica.update(t)?,
            SettingListType::Debug => self.list_debug.update(t)?,
            SettingListType::About => false,
        };
        self.scroll.update(t);
        if changed {
            self.save_time = t;
        }
        if t > self.save_time + Self::SAVE_TIME {
            save_data()?;
            self.save_time = f32::INFINITY;
        }
        Ok(())
    }

    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        let t = s.t;
        let rt = s.rt;

        s.fader.render(ui, s.t, |ui| {
            let r = ui.content_rect();
            self.tabs.render(ui, rt, r, |ui, item| {
                let r = r.feather(-0.01);
                self.scroll.size((r.w, r.h));
                ui.scope(|ui| {
                    ui.dx(r.x);
                    ui.dy(r.y);
                    self.scroll.render(ui, |ui| match item {
                        SettingListType::General => self.list_general.render(ui, r, t),
                        SettingListType::Audio => self.list_audio.render(ui, r, t),
                        SettingListType::Chart => self.list_chart.render(ui, r, t),
                        SettingListType::Replica => self.list_replica.render(ui, r, t),
                        SettingListType::Debug => self.list_debug.render(ui, r, t),
                        SettingListType::About => render_about(ui, r, &self.icon),
                    });
                });

                Ok(())
            })
        })?;

        Ok(())
    }

    fn next_page(&mut self) -> NextPage {
        if matches!(self.tabs.selected(), SettingListType::Audio) {
            return self.list_audio.next_page().unwrap_or_default();
        }
        NextPage::None
    }
}

fn render_about(ui: &mut Ui, mut r: Rect, icon: &SafeTexture) -> (f32, f32) {
    r.x = 0.;
    r.y = 0.;
    let ow = r.w;
    let r = r.feather(-0.02);

    let ct = r.center();
    let s = 0.1;
    let ir = Rect::new(ct.x - s, r.y + 0.05, s * 2., s * 2.);
    ui.fill_path(&ir.rounded(0.02), (**icon, ir));

    let staff = &*STAFF_LIST;
    let text = tl!(
        "about-content",
        "version" => format!("{} ({})", env!("CARGO_PKG_VERSION"), env!("GIT_HASH")),

        "development" => &staff.development.0,
        "operations" => &staff.operations.0,
        "documentation" => &staff.documentation.0,
        "art" => &staff.art.0,
        "music" => &staff.music.0,
        "audio" => &staff.audio.0,
        "community" => &staff.community.0,
        "localization" => &staff.localization.0
    );
    let (first, text) = text.split_once('\n').unwrap();
    let tr = ui
        .text(first)
        .pos(ct.x, ir.bottom() + 0.03)
        .anchor(0.5, 0.)
        .size(0.6)
        .draw_using(&BOLD_FONT);

    let r = ui
        .text(text.trim())
        .pos(r.x, tr.bottom() + 0.06)
        .size(0.55)
        .multiline()
        .max_width(r.w)
        .h_center()
        .draw();

    (ow, r.bottom() + 0.03)
}

fn render_title<'a>(ui: &mut Ui, title: impl Into<Cow<'a, str>>, subtitle: Option<Cow<'a, str>>) -> f32 {
    const TITLE_SIZE: f32 = 0.6;
    const SUBTITLE_SIZE: f32 = 0.35;
    const LEFT: f32 = 0.06;
    const PAD: f32 = 0.01;
    const SUB_MAX_WIDTH: f32 = 1.4;
    if let Some(subtitle) = subtitle {
        let title = title.into();
        let r1 = ui.text(Cow::clone(&title)).size(TITLE_SIZE).measure();
        let r2 = ui
            .text(Cow::clone(&subtitle))
            .size(SUBTITLE_SIZE)
            .max_width(SUB_MAX_WIDTH)
            .no_baseline()
            .measure();
        let h = r1.h + PAD + r2.h;
        let r1 = ui
            .text(subtitle)
            .pos(LEFT, (ITEM_HEIGHT + h) / 2.)
            .anchor(0., 1.)
            .size(SUBTITLE_SIZE)
            .max_width(SUB_MAX_WIDTH)
            .color(semi_white(0.6))
            .draw()
            .right();
        let r2 = ui
            .text(title)
            .pos(LEFT, (ITEM_HEIGHT - h) / 2.)
            .no_baseline()
            .size(TITLE_SIZE)
            .draw()
            .right();
        r1.max(r2)
    } else {
        ui.text(title.into())
            .pos(LEFT, ITEM_HEIGHT / 2.)
            .anchor(0., 0.5)
            .no_baseline()
            .size(TITLE_SIZE)
            .draw()
            .right()
    }
}

#[inline]
fn render_switch(ui: &mut Ui, r: Rect, t: f32, btn: &mut DRectButton, on: bool) {
    btn.render_text(ui, r, t, if on { ttl!("switch-on") } else { ttl!("switch-off") }, 0.5, on);
}

#[inline]
fn right_rect(w: f32) -> Rect {
    let rh = ITEM_HEIGHT * 2. / 3.;
    Rect::new(w - 0.3, (ITEM_HEIGHT - rh) / 2., INTERACT_WIDTH, rh)
}

struct GeneralList {
    icon_lang: SafeTexture,

    lang_btn: ChooseButton,

    #[cfg(all(any(target_os = "windows", target_os = "linux"), not(target_env = "ohos")))]
    fullscreen_btn: DRectButton,

    cache_btn: DRectButton,
    offline_btn: DRectButton,
    server_status_btn: DRectButton,
    mp_btn: DRectButton,
    mp_addr_btn: DRectButton,
    #[cfg(not(target_env = "ohos"))]
    lowq_btn: DRectButton,
    prefer_reduced_motion_btn: DRectButton,
    insecure_btn: DRectButton,
    enable_anys_btn: DRectButton,
    anys_gateway_btn: DRectButton,

    cache_size: Option<u64>,
    cache_task: Option<Task<Result<u64>>>,
}

impl GeneralList {
    pub fn new(icon_lang: SafeTexture) -> Self {
        let mut this = Self {
            icon_lang,

            lang_btn: ChooseButton::new()
                .with_options(LANG_NAMES.iter().map(|s| s.to_string()).collect())
                .with_selected(
                    get_data()
                        .language
                        .as_ref()
                        .and_then(|it| it.parse::<LanguageIdentifier>().ok())
                        .and_then(|ident| LANG_IDENTS.iter().position(|it| *it == ident))
                        .unwrap_or_default(),
                ),

            #[cfg(all(any(target_os = "windows", target_os = "linux"), not(target_env = "ohos")))]
            fullscreen_btn: DRectButton::new(),

            cache_btn: DRectButton::new(),
            offline_btn: DRectButton::new(),
            server_status_btn: DRectButton::new(),
            mp_btn: DRectButton::new(),
            mp_addr_btn: DRectButton::new(),
            #[cfg(not(target_env = "ohos"))]
            lowq_btn: DRectButton::new(),
            prefer_reduced_motion_btn: DRectButton::new(),
            insecure_btn: DRectButton::new(),
            enable_anys_btn: DRectButton::new(),
            anys_gateway_btn: DRectButton::new(),

            cache_size: None,
            cache_task: None,
        };
        let _ = this.update_cache_size();
        this
    }

    pub fn top_touch(&mut self, touch: &Touch, t: f32) -> bool {
        if self.lang_btn.top_touch(touch, t) {
            return true;
        }
        false
    }

    fn dir_size(path: impl Into<PathBuf>) -> io::Result<u64> {
        fn inner(mut dir: fs::ReadDir) -> io::Result<u64> {
            dir.try_fold(0, |acc, file| {
                let file = file?;
                let size = match file.metadata()? {
                    data if data.is_dir() => inner(fs::read_dir(file.path())?)?,
                    data => data.len(),
                };
                Ok(acc + size)
            })
        }

        inner(fs::read_dir(path.into())?)
    }

    fn update_cache_size(&mut self) -> Result<()> {
        self.cache_size = None;

        let cache_dir = dir::cache()?;
        self.cache_task = Some(Task::new(async { Ok(Self::dir_size(cache_dir)?) }));
        Ok(())
    }

    pub fn touch(&mut self, touch: &Touch, t: f32) -> Result<Option<bool>> {
        let data = get_data_mut();
        let config = &mut data.config;
        if self.lang_btn.touch(touch, t) {
            return Ok(Some(false));
        }

        #[cfg(all(any(target_os = "windows", target_os = "linux"), not(target_env = "ohos")))]
        if self.fullscreen_btn.touch(touch, t) {
            config.fullscreen_mode ^= true;

            macroquad::window::set_fullscreen(config.fullscreen_mode);

            return Ok(Some(true));
        }

        if self.cache_btn.touch(touch, t) {
            fs::remove_dir_all(dir::cache()?)?;
            self.update_cache_size()?;
            show_message(tl!("item-cache-cleared")).ok();
            return Ok(Some(false));
        }
        if self.offline_btn.touch(touch, t) {
            config.offline_mode ^= true;
            return Ok(Some(true));
        }
        if self.server_status_btn.touch(touch, t) {
            let _ = open_url(STATUS_PAGE);
            return Ok(Some(true));
        }
        if self.mp_btn.touch(touch, t) {
            config.mp_enabled ^= true;
            return Ok(Some(true));
        }
        if self.mp_addr_btn.touch(touch, t) {
            request_input("mp_addr", InputBox::new().default_text(&config.mp_address));
            return Ok(Some(true));
        }
        #[cfg(not(target_env = "ohos"))]
        if self.lowq_btn.touch(touch, t) {
            config.sample_count = if config.sample_count == 1 { 2 } else { 1 };
            return Ok(Some(true));
        }
        if self.prefer_reduced_motion_btn.touch(touch, t) {
            data.prefer_reduced_motion ^= true;
            PREFER_REDUCED_MOTION.store(data.prefer_reduced_motion, Ordering::Relaxed);
            return Ok(Some(true));
        }
        if self.insecure_btn.touch(touch, t) {
            data.accept_invalid_cert ^= true;
            return Ok(Some(true));
        }
        if self.enable_anys_btn.touch(touch, t) {
            data.enable_anys ^= true;
            return Ok(Some(true));
        }
        if self.anys_gateway_btn.touch(touch, t) {
            request_input("anys_gateway", InputBox::new().default_text(&data.anys_gateway));
            return Ok(Some(true));
        }
        Ok(None)
    }

    pub fn update(&mut self, t: f32) -> Result<bool> {
        self.lang_btn.update(t);
        let data = get_data_mut();
        if self.lang_btn.changed() {
            data.language = Some(LANG_IDENTS[self.lang_btn.selected()].to_string());
            sync_data();
            return Ok(true);
        }
        if let Some((id, text)) = take_input() {
            if id == "mp_addr" {
                if let Err(err) = text.parse::<http::uri::Authority>() {
                    show_error(anyhow::Error::new(err).context(tl!("item-mp-addr-invalid")));
                    return Ok(false);
                } else {
                    data.config.mp_address = text;
                    return Ok(true);
                }
            } else if id == "anys_gateway" {
                if let Err(err) = Url::parse(&text) {
                    show_error(anyhow::Error::new(err).context(tl!("item-anys-gateway-invalid")));
                    return Ok(false);
                } else {
                    data.anys_gateway = text.trim_end_matches('/').to_string();
                    return Ok(true);
                }
            } else {
                return_input(id, text);
            }
        }
        if let Some(task) = &mut self.cache_task {
            if let Some(size) = task.take() {
                self.cache_size = size.ok();
                self.cache_task = None;
            }
        }
        Ok(false)
    }

    pub fn render(&mut self, ui: &mut Ui, r: Rect, t: f32) -> (f32, f32) {
        let w = r.w;
        let mut h = 0.;
        macro_rules! item {
            ($($b:tt)*) => {{
                $($b)*
                ui.dy(ITEM_HEIGHT);
                h += ITEM_HEIGHT;
            }}
        }
        let rr = right_rect(w);

        let data = get_data();
        let config = &data.config;
        item! {
            let rt = render_title(ui, tl!("item-lang"), None);
            let w = 0.06;
            let r = Rect::new(rt + 0.01, (ITEM_HEIGHT - w) / 2., w, w);
            ui.fill_rect(r, (*self.icon_lang, r));
            self.lang_btn.render(ui, rr, t);
        }

        #[cfg(all(any(target_os = "windows", target_os = "linux"), not(target_env = "ohos")))]
        item! {
            render_title(ui, tl!("item-fullscreen"), None);
            render_switch(ui, rr, t, &mut self.fullscreen_btn, config.fullscreen_mode);
        }

        item! {
            render_title(ui, tl!("item-offline"), Some(tl!("item-offline-sub")));
            render_switch(ui, rr, t, &mut self.offline_btn, config.offline_mode);
        }
        item! {
            render_title(ui, tl!("item-server-status"), Some(tl!("item-server-status-sub")));
            self.server_status_btn.render_text(ui, rr, t, tl!("check-status"), 0.5, true);
        }
        item! {
            render_title(ui, tl!("item-mp"), Some(tl!("item-mp-sub")));
            render_switch(ui, rr, t, &mut self.mp_btn, config.mp_enabled);
        }
        item! {
            render_title(ui, tl!("item-mp-addr"), Some(tl!("item-mp-addr-sub")));
            self.mp_addr_btn.render_text(ui, rr, t, &config.mp_address, 0.4, false);
        }
        item! {
            render_title(ui, tl!("item-prefer-reduced-motion"), Some(tl!("item-prefer-reduced-motion-sub")));
            render_switch(ui, rr, t, &mut self.prefer_reduced_motion_btn, data.prefer_reduced_motion);
        }
        #[cfg(not(target_env = "ohos"))]
        item! {
            render_title(ui, tl!("item-lowq"), Some(tl!("item-lowq-sub")));
            render_switch(ui, rr, t, &mut self.lowq_btn, config.sample_count == 1);
        }
        item! {
            let cache_size = if let Some(size) = self.cache_size {
                Cow::Owned(tl!("item-cache-size", "size" => ByteSize(size).to_string()))
            } else {
                tl!("item-cache-size-loading")
            };
            render_title(ui, tl!("item-clear-cache"), Some(cache_size));
            self.cache_btn.render_text(ui, rr, t, tl!("item-clear-cache-btn"), 0.5, true);
        }
        ui.dy(0.04);
        h += 0.04;
        item! {
            render_title(ui, tl!("item-insecure"), Some(tl!("item-insecure-sub")));
            render_switch(ui, rr, t, &mut self.insecure_btn, data.accept_invalid_cert);
        }
        item! {
            render_title(ui, tl!("item-enable-anys"), Some(tl!("item-enable-anys-sub")));
            render_switch(ui, rr, t, &mut self.enable_anys_btn, data.enable_anys);
        }
        item! {
            render_title(ui, tl!("item-anys-gateway"), Some(tl!("item-anys-gateway-sub")));
            self.anys_gateway_btn.render_text(ui, rr, t, &data.anys_gateway, 0.4, false);
        }
        self.lang_btn.render_top(ui, t, 1.);
        (w, h)
    }
}

struct AudioList {
    adjust_btn: DRectButton,
    music_slider: Slider,
    sfx_slider: Slider,
    bgm_slider: Slider,
    cali_btn: DRectButton,
    #[cfg(not(target_os = "android"))]
    preferred_sample_rate_btn: DRectButton,
    #[cfg(target_env = "ohos")]
    audio_buffer_size_btn: DRectButton,
    cali_task: LocalTask<Result<OffsetPage>>,
    next_page: Option<NextPage>,
}

impl AudioList {
    pub fn new() -> Self {
        Self {
            adjust_btn: DRectButton::new(),
            music_slider: Slider::new(0.0..2.0, 0.05),
            sfx_slider: Slider::new(0.0..2.0, 0.05),
            bgm_slider: Slider::new(0.0..2.0, 0.05),
            cali_btn: DRectButton::new(),
            #[cfg(not(target_os = "android"))]
            preferred_sample_rate_btn: DRectButton::new(),
            #[cfg(target_env = "ohos")]
            audio_buffer_size_btn: DRectButton::new(),

            cali_task: None,
            next_page: None,
        }
    }

    pub fn top_touch(&mut self, _touch: &Touch, _t: f32) -> bool {
        false
    }

    pub fn touch(&mut self, touch: &Touch, t: f32) -> Result<Option<bool>> {
        let data = get_data_mut();
        let config = &mut data.config;
        if self.adjust_btn.touch(touch, t) {
            config.adjust_time ^= true;
            return Ok(Some(true));
        }
        if let wt @ Some(_) = self.music_slider.touch(touch, t, &mut config.volume_music) {
            return Ok(wt);
        }
        if let wt @ Some(_) = self.sfx_slider.touch(touch, t, &mut config.volume_sfx) {
            UI_SFX_VOLUME.store(config.volume_sfx.to_bits(), Ordering::Relaxed);
            return Ok(wt);
        }
        let old = config.volume_bgm;
        if let wt @ Some(_) = self.bgm_slider.touch(touch, t, &mut config.volume_bgm) {
            if (config.volume_bgm - old).abs() > 0.001 {
                BGM_VOLUME_UPDATED.store(true, Ordering::Relaxed);
            }
            return Ok(wt);
        }
        if self.cali_btn.touch(touch, t) {
            self.cali_task = Some(Box::pin(OffsetPage::new()));
            return Ok(Some(false));
        }
        #[cfg(not(target_os = "android"))]
        if self.preferred_sample_rate_btn.touch(touch, t) {
            let options = [None, Some(44100), Some(48000), Some(88200), Some(96000), Some(192000)];
            let current = config.preferred_sample_rate;
            let selected = options.iter().position(|&r| r == current).unwrap_or(0);
            config.preferred_sample_rate = options[(selected + 1) % options.len()];
            return Ok(Some(true));
        }
        #[cfg(target_env = "ohos")]
        if self.audio_buffer_size_btn.touch(touch, t) {
            let options = [128u32, 256u32, 512u32];
            let current = config.audio_buffer_size.unwrap_or(256);
            let selected = options.iter().position(|&r| r == current).unwrap_or(1);
            config.audio_buffer_size = Some(options[(selected + 1) % options.len()]);
            return Ok(Some(true));
        }
        Ok(None)
    }

    pub fn update(&mut self, _t: f32) -> Result<bool> {
        if let Some(task) = &mut self.cali_task {
            if let Some(res) = poll_future(task.as_mut()) {
                match res {
                    Err(err) => show_error(err.context(tl!("load-cali-failed"))),
                    Ok(page) => {
                        self.next_page = Some(NextPage::Overlay(Box::new(page)));
                    }
                }
                self.cali_task = None;
            }
        }
        Ok(false)
    }

    pub fn render(&mut self, ui: &mut Ui, r: Rect, t: f32) -> (f32, f32) {
        let w = r.w;
        let mut h = 0.;
        macro_rules! item {
            ($($b:tt)*) => {{
                $($b)*
                ui.dy(ITEM_HEIGHT);
                h += ITEM_HEIGHT;
            }}
        }
        let rr = right_rect(w);

        let data = get_data();
        let config = &data.config;
        item! {
            render_title(ui, tl!("item-adjust"), Some(tl!("item-adjust-sub")));
            render_switch(ui, rr, t, &mut self.adjust_btn, config.adjust_time);
        }
        item! {
            render_title(ui, tl!("item-music"), None);
            self.music_slider.render(ui, rr, t, config.volume_music, format!("{:.2}", config.volume_music));
        }
        item! {
            render_title(ui, tl!("item-sfx"), None);
            self.sfx_slider.render(ui, rr, t, config.volume_sfx, format!("{:.2}", config.volume_sfx));
        }
        item! {
            render_title(ui, tl!("item-bgm"), None);
            self.bgm_slider.render(ui, rr, t, config.volume_bgm, format!("{:.2}", config.volume_bgm));
        }
        item! {
            render_title(ui, tl!("item-cali"), None);
            self.cali_btn.render_text(ui, rr, t, format!("{:.0}ms", config.offset * 1000.), 0.5, true);
        }
        #[cfg(not(target_os = "android"))]
        item! {
            render_title(ui, tl!("item-preferred-sample-rate"), None);
            let text = if let Some(rate) = config.preferred_sample_rate {
                format!("{} Hz", rate)
            } else {
                tl!("preferred-sample-rate-default").to_string()
            };
            self.preferred_sample_rate_btn.render_text(ui, rr, t, text, 0.5, false);
        }
        #[cfg(target_env = "ohos")]
        item! {
            render_title(ui, tl!("item-audio-buffer-size"), None);
            let buf_size = config.audio_buffer_size.unwrap_or(256);
            self.audio_buffer_size_btn.render_text(ui, rr, t, format!("{}", buf_size), 0.5, false);
        }
        (w, h)
    }

    pub fn next_page(&mut self) -> Option<NextPage> {
        self.next_page.take()
    }
}

struct ChartList {
    replica: bool,
    judge_mode_btn: ChooseButton,
    phigros_strict_judge_btn: DRectButton,
    shorten_holds_btn: DRectButton,
    ai_auto_btn: DRectButton,
    auto_flip_btn: DRectButton,
    hold_head_effect_btn: DRectButton,
    show_acc_btn: DRectButton,
    ap_fc_indicator_btn: DRectButton,
    show_avg_fps_btn: DRectButton,
    dc_pause_btn: DRectButton,
    dhint_btn: DRectButton,
    opt_btn: DRectButton,
    use_keyboard_btn: DRectButton,
    size_slider: Slider,
}

impl ChartList {
    pub fn new(replica: bool) -> Self {
        Self {
            replica,
            judge_mode_btn: ChooseButton::new()
                .with_options(vec![tl!("judgement-mode-phira").into_owned(), tl!("judgement-mode-phigros").into_owned()])
                .with_selected(match get_data().config.judgement_mode {
                    JudgementMode::Phira => 0,
                    JudgementMode::PhigrosReplica => 1,
                }),
            phigros_strict_judge_btn: DRectButton::new(),
            shorten_holds_btn: DRectButton::new(),
            ai_auto_btn: DRectButton::new(),
            auto_flip_btn: DRectButton::new(),
            hold_head_effect_btn: DRectButton::new(),
            show_acc_btn: DRectButton::new(),
            ap_fc_indicator_btn: DRectButton::new(),
            show_avg_fps_btn: DRectButton::new(),
            dc_pause_btn: DRectButton::new(),
            dhint_btn: DRectButton::new(),
            opt_btn: DRectButton::new(),
            use_keyboard_btn: DRectButton::new(),
            size_slider: Slider::new(0.8..1.2, 0.005),
        }
    }

    pub fn top_touch(&mut self, touch: &Touch, t: f32) -> bool {
        self.replica && self.judge_mode_btn.top_touch(touch, t)
    }

    pub fn touch(&mut self, touch: &Touch, t: f32) -> Result<Option<bool>> {
        let data = get_data_mut();
        let config = &mut data.config;
        if self.replica && self.judge_mode_btn.touch(touch, t) {
            return Ok(Some(false));
        }
        if self.replica && self.hold_head_effect_btn.touch(touch, t) {
            config.hold_head_effect ^= true;
            return Ok(Some(true));
        }
        if self.replica && self.auto_flip_btn.touch(touch, t) {
            config.auto_flip_enabled ^= true;
            return Ok(Some(true));
        }
        if self.replica && self.ai_auto_btn.touch(touch, t) {
            config.ai_auto_predict ^= true;
            return Ok(Some(true));
        }
        if self.replica && self.shorten_holds_btn.touch(touch, t) {
            config.shorten_holds ^= true;
            return Ok(Some(true));
        }
        if self.replica && config.has_custom_judgement() && self.phigros_strict_judge_btn.touch(touch, t) {
            config.phigros_strict_judgement ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.show_acc_btn.touch(touch, t) {
            config.show_acc ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.ap_fc_indicator_btn.touch(touch, t) {
            config.ap_fc_indicator ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.show_avg_fps_btn.touch(touch, t) {
            config.show_avg_fps ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.dc_pause_btn.touch(touch, t) {
            config.double_click_to_pause ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.dhint_btn.touch(touch, t) {
            config.double_hint ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.opt_btn.touch(touch, t) {
            config.aggressive ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.use_keyboard_btn.touch(touch, t) {
            config.use_keyboard ^= true;
            return Ok(Some(true));
        }
        if !self.replica {
            if let wt @ Some(_) = self.size_slider.touch(touch, t, &mut config.note_scale) {
                return Ok(wt);
            }
        }
        Ok(None)
    }

    pub fn update(&mut self, t: f32) -> Result<bool> {
        self.judge_mode_btn.update(t);
        if self.judge_mode_btn.changed() {
            get_data_mut().config.judgement_mode = match self.judge_mode_btn.selected() {
                0 => JudgementMode::Phira,
                _ => JudgementMode::PhigrosReplica,
            };
            return Ok(true);
        }
        Ok(false)
    }

    pub fn render(&mut self, ui: &mut Ui, r: Rect, t: f32) -> (f32, f32) {
        let w = r.w;
        let mut h = 0.;
        macro_rules! item {
            ($width:expr; $($b:tt)*) => {{
                if self.replica { ui.fill_path(&Rect::new(0., 0., $width, ITEM_HEIGHT - 0.012).rounded(0.012), Color::from_rgba(30, 49, 62, 180)); }
                $($b)*
                ui.dy(ITEM_HEIGHT);
                h += ITEM_HEIGHT;
            }}
        }
        let rr = right_rect(w);

        let data = get_data();
        let config = &data.config;
        if self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-judgement-mode"), Some(tl!("item-judgement-mode-sub")));
                self.judge_mode_btn.render(ui, rr, t);
            }
        }
        if self.replica && config.has_custom_judgement() {
            ui.dx(0.035);
            let w = w - 0.035;
            let rr = right_rect(w);
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-phigros-strict-judgement"), Some(tl!("item-phigros-strict-judgement-sub")));
                render_switch(ui, rr, t, &mut self.phigros_strict_judge_btn, config.phigros_strict_judgement);
            }
            ui.dx(-0.035);
        }
        if self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-hold-head-effect"), Some(tl!("item-hold-head-effect-sub")));
                render_switch(ui, rr, t, &mut self.hold_head_effect_btn, config.hold_head_effect);
            }
        }
        if self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-shorten-holds"), Some(tl!("item-shorten-holds-sub")));
                render_switch(ui, rr, t, &mut self.shorten_holds_btn, config.shorten_holds);
            }
        }
        if self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-auto-flip"), Some(tl!("item-auto-flip-sub")));
                render_switch(ui, rr, t, &mut self.auto_flip_btn, config.auto_flip_enabled);
            }
        }
        if self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-ai-auto"), Some(tl!("item-ai-auto-sub")));
                render_switch(ui, rr, t, &mut self.ai_auto_btn, config.ai_auto_predict);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-show-acc"), None);
                render_switch(ui, rr, t, &mut self.show_acc_btn, config.show_acc);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-ap-fc-indicator"), Some(tl!("item-ap-fc-indicator-sub")));
                render_switch(ui, rr, t, &mut self.ap_fc_indicator_btn, config.ap_fc_indicator);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-show-avg-fps"), Some(tl!("item-show-avg-fps-sub")));
                render_switch(ui, rr, t, &mut self.show_avg_fps_btn, config.show_avg_fps);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-dc-pause"), None);
                render_switch(ui, rr, t, &mut self.dc_pause_btn, config.double_click_to_pause);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-dhint"), Some(tl!("item-dhint-sub")));
                render_switch(ui, rr, t, &mut self.dhint_btn, config.double_hint);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-opt"), Some(tl!("item-opt-sub")));
                render_switch(ui, rr, t, &mut self.opt_btn, config.aggressive);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-use-keyboard"), Some(tl!("item-use-keyboard-sub")));
                render_switch(ui, rr, t, &mut self.use_keyboard_btn, config.use_keyboard);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-note-size"), None);
                self.size_slider.render(ui, rr, t, config.note_scale, format!("{:.3}", config.note_scale));
            }
        }
        if self.replica {
            self.judge_mode_btn.render_top(ui, t, 1.);
        }
        (w, h)
    }
}

struct DebugList {
    replica: bool,
    ranges: bool,
    reports: bool,
    chart_debug_btn: DRectButton,
    touch_debug_btn: DRectButton,
    play_report_btn: DRectButton,
    touch_report_debug_btn: DRectButton,
    judgement_range_btn: DRectButton,
    judgement_range_anchor_btn: ChooseButton,
    judgement_range_horizon_slider: Slider,
    judgement_range_alpha_slider: Slider,
    tap_range_btn: DRectButton,
    tap_layer_btns: [DRectButton; 4],
    hold_range_btn: DRectButton,
    hold_layer_btns: [DRectButton; 4],
    drag_range_btn: DRectButton,
    flick_range_btn: DRectButton,
}

impl DebugList {
    pub fn new(replica: bool) -> Self {
        Self {
            replica,
            ranges: replica,
            reports: replica,
            chart_debug_btn: DRectButton::new(),
            touch_debug_btn: DRectButton::new(),
            play_report_btn: DRectButton::new(),
            touch_report_debug_btn: DRectButton::new(),
            judgement_range_btn: DRectButton::new(),
            judgement_range_anchor_btn: ChooseButton::new()
                .with_options(vec![
                    tl!("judgement-range-anchor-line").into_owned(),
                    tl!("judgement-range-anchor-note").into_owned(),
                    tl!("judgement-range-anchor-both").into_owned(),
                ])
                .with_selected(match get_data().config.judgement_range_debug.anchor {
                    JudgementRangeAnchor::JudgeLine => 0,
                    JudgementRangeAnchor::Note => 1,
                    JudgementRangeAnchor::Both => 2,
                }),
            judgement_range_horizon_slider: Slider::new(JUDGEMENT_RANGE_HORIZON_MIN..JUDGEMENT_RANGE_HORIZON_MAX, JUDGEMENT_RANGE_HORIZON_STEP),
            judgement_range_alpha_slider: Slider::new(JUDGEMENT_RANGE_ALPHA_MIN..JUDGEMENT_RANGE_ALPHA_MAX, JUDGEMENT_RANGE_ALPHA_STEP),
            tap_range_btn: DRectButton::new(),
            tap_layer_btns: std::array::from_fn(|_| DRectButton::new()),
            hold_range_btn: DRectButton::new(),
            hold_layer_btns: std::array::from_fn(|_| DRectButton::new()),
            drag_range_btn: DRectButton::new(),
            flick_range_btn: DRectButton::new(),
        }
    }

    fn range_settings() -> Self {
        Self {
            reports: false,
            ..Self::new(true)
        }
    }
    fn report_settings() -> Self {
        Self {
            ranges: false,
            ..Self::new(true)
        }
    }

    pub fn top_touch(&mut self, touch: &Touch, t: f32) -> bool {
        self.ranges && get_data().config.judgement_range_debug.enabled && self.judgement_range_anchor_btn.top_touch(touch, t)
    }

    pub fn touch(&mut self, touch: &Touch, t: f32) -> Result<Option<bool>> {
        let data = get_data_mut();
        let config = &mut data.config;
        if !self.replica && self.chart_debug_btn.touch(touch, t) {
            config.chart_debug ^= true;
            return Ok(Some(true));
        }
        if !self.replica && self.touch_debug_btn.touch(touch, t) {
            config.touch_debug ^= true;
            return Ok(Some(true));
        }
        if self.reports && self.play_report_btn.touch(touch, t) {
            config.auto_export_play_report ^= true;
            return Ok(Some(true));
        }
        if self.reports && self.touch_report_debug_btn.touch(touch, t) {
            config.touch_input_debug_report ^= true;
            return Ok(Some(true));
        }
        if self.ranges && self.judgement_range_btn.touch(touch, t) {
            config.judgement_range_debug.enabled ^= true;
            return Ok(Some(true));
        }
        if !self.ranges || !config.judgement_range_debug.enabled {
            return Ok(None);
        }
        if self.judgement_range_anchor_btn.touch(touch, t) {
            return Ok(Some(false));
        }
        if let wt @ Some(_) = self
            .judgement_range_horizon_slider
            .touch(touch, t, &mut config.judgement_range_debug.horizon)
        {
            return Ok(wt);
        }
        if let wt @ Some(_) = self
            .judgement_range_alpha_slider
            .touch(touch, t, &mut config.judgement_range_debug.fill_alpha)
        {
            return Ok(wt);
        }
        if self.tap_range_btn.touch(touch, t) {
            config.judgement_range_debug.tap.enabled ^= true;
            return Ok(Some(true));
        }
        if config.judgement_range_debug.tap.enabled {
            let layers = &mut config.judgement_range_debug.tap;
            let values = [&mut layers.perfect, &mut layers.good, &mut layers.bad, &mut layers.miss];
            for (btn, value) in self.tap_layer_btns.iter_mut().zip(values) {
                if btn.touch(touch, t) {
                    *value ^= true;
                    return Ok(Some(true));
                }
            }
        }
        if self.hold_range_btn.touch(touch, t) {
            config.judgement_range_debug.hold.enabled ^= true;
            return Ok(Some(true));
        }
        if config.judgement_range_debug.hold.enabled {
            let layers = &mut config.judgement_range_debug.hold;
            let values = [&mut layers.head_perfect, &mut layers.head_good, &mut layers.head_miss, &mut layers.tail];
            for (btn, value) in self.hold_layer_btns.iter_mut().zip(values) {
                if btn.touch(touch, t) {
                    *value ^= true;
                    return Ok(Some(true));
                }
            }
        }
        if self.drag_range_btn.touch(touch, t) {
            config.judgement_range_debug.drag ^= true;
            return Ok(Some(true));
        }
        if self.flick_range_btn.touch(touch, t) {
            config.judgement_range_debug.flick ^= true;
            return Ok(Some(true));
        }
        Ok(None)
    }

    pub fn update(&mut self, t: f32) -> Result<bool> {
        self.judgement_range_anchor_btn.update(t);
        if self.judgement_range_anchor_btn.changed() {
            get_data_mut().config.judgement_range_debug.anchor = match self.judgement_range_anchor_btn.selected() {
                0 => JudgementRangeAnchor::JudgeLine,
                1 => JudgementRangeAnchor::Note,
                _ => JudgementRangeAnchor::Both,
            };
            return Ok(true);
        }
        Ok(false)
    }

    fn render_layer_buttons(ui: &mut Ui, w: f32, t: f32, btns: &mut [DRectButton; 4], labels: [&str; 4], values: [bool; 4]) {
        let total_w = w - 0.07;
        let gap = 0.012;
        let bh = ITEM_HEIGHT * 0.54;
        let bw = (total_w - gap * 3.) / 4.;
        let mut x = 0.035;
        let y = (ITEM_HEIGHT - bh) / 2.;
        for ((btn, label), value) in btns.iter_mut().zip(labels).zip(values) {
            btn.render_text(ui, Rect::new(x, y, bw, bh), t, label, 0.42, value);
            x += bw + gap;
        }
    }

    pub fn render(&mut self, ui: &mut Ui, r: Rect, t: f32) -> (f32, f32) {
        let w = r.w;
        let mut h = 0.;
        macro_rules! item {
            ($width:expr; $($b:tt)*) => {{
                if self.replica { ui.fill_path(&Rect::new(0., 0., $width, ITEM_HEIGHT - 0.012).rounded(0.012), Color::from_rgba(30, 49, 62, 180)); }
                $($b)*
                ui.dy(ITEM_HEIGHT);
                h += ITEM_HEIGHT;
            }}
        }
        let rr = right_rect(w);

        let data = get_data();
        let config = &data.config;
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-chart-debug"), Some(tl!("item-chart-debug-sub")));
                render_switch(ui, rr, t, &mut self.chart_debug_btn, config.chart_debug);
            }
        }
        if !self.replica {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-touch-debug"), Some(tl!("item-touch-debug-sub")));
                render_switch(ui, rr, t, &mut self.touch_debug_btn, config.touch_debug);
            }
        }
        if self.reports {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-play-report"), Some(tl!("item-play-report-sub")));
                render_switch(ui, rr, t, &mut self.play_report_btn, config.auto_export_play_report);
            }
        }
        if self.reports {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-touch-report-debug"), Some(tl!("item-touch-report-debug-sub")));
                render_switch(ui, rr, t, &mut self.touch_report_debug_btn, config.touch_input_debug_report);
            }
        }
        if self.ranges {
            item! { w;
                render_partition_title(ui, w, self.replica, tl!("item-judgement-range-debug"), Some(tl!("item-judgement-range-debug-sub")));
                render_switch(ui, rr, t, &mut self.judgement_range_btn, config.judgement_range_debug.enabled);
            }
        }
        if self.ranges && config.judgement_range_debug.enabled {
            ui.dx(0.035);
            let w = w - 0.035;
            let rr = right_rect(w);
            if self.replica {
                item! { w;
                    render_partition_title(ui, w, self.replica, tl!("item-judgement-range-anchor"), Some(tl!("item-judgement-range-anchor-sub")));
                    self.judgement_range_anchor_btn.render(ui, rr, t);
                }
            }
            if self.replica {
                item! { w;
                    render_partition_title(ui, w, self.replica, tl!("item-judgement-range-horizon"), Some(tl!("item-judgement-range-horizon-sub")));
                    self.judgement_range_horizon_slider.render(
                        ui,
                        rr,
                        t,
                        config.judgement_range_debug.horizon,
                        format!("{:.1}s", config.judgement_range_debug.horizon),
                    );
                }
            }
            if self.replica {
                item! { w;
                    render_partition_title(ui, w, self.replica, tl!("item-judgement-range-alpha"), Some(tl!("item-judgement-range-alpha-sub")));
                    self.judgement_range_alpha_slider.render(
                        ui,
                        rr,
                        t,
                        config.judgement_range_debug.fill_alpha,
                        format!("{:.0}%", config.judgement_range_debug.fill_alpha * 100.),
                    );
                }
            }
            if self.replica {
                item! { w;
                    render_partition_title(ui, w, self.replica, tl!("item-judgement-range-tap"), Some(tl!("item-judgement-range-tap-sub")));
                    render_switch(ui, rr, t, &mut self.tap_range_btn, config.judgement_range_debug.tap.enabled);
                }
            }
            if config.judgement_range_debug.tap.enabled {
                ui.dx(0.035);
                let w = w - 0.035;
                if self.replica {
                    item! { w;
                        Self::render_layer_buttons(
                            ui,
                            w,
                            t,
                            &mut self.tap_layer_btns,
                            ["Perfect", "Good", "Bad", "Miss"],
                            [
                                config.judgement_range_debug.tap.perfect,
                                config.judgement_range_debug.tap.good,
                                config.judgement_range_debug.tap.bad,
                                config.judgement_range_debug.tap.miss,
                            ],
                        );
                    }
                }
                ui.dx(-0.035);
            }
            if self.replica {
                item! { w;
                    render_partition_title(ui, w, self.replica, tl!("item-judgement-range-hold"), Some(tl!("item-judgement-range-hold-sub")));
                    render_switch(ui, rr, t, &mut self.hold_range_btn, config.judgement_range_debug.hold.enabled);
                }
            }
            if config.judgement_range_debug.hold.enabled {
                ui.dx(0.035);
                let w = w - 0.035;
                if self.replica {
                    item! { w;
                        Self::render_layer_buttons(
                            ui,
                            w,
                            t,
                            &mut self.hold_layer_btns,
                            ["Perfect", "Good", "Miss", "Tail"],
                            [
                                config.judgement_range_debug.hold.head_perfect,
                                config.judgement_range_debug.hold.head_good,
                                config.judgement_range_debug.hold.head_miss,
                                config.judgement_range_debug.hold.tail,
                            ],
                        );
                    }
                }
                ui.dx(-0.035);
            }
            if self.replica {
                item! { w;
                    render_partition_title(ui, w, self.replica, tl!("item-judgement-range-drag"), Some(tl!("item-judgement-range-drag-sub")));
                    render_switch(ui, rr, t, &mut self.drag_range_btn, config.judgement_range_debug.drag);
                }
            }
            if self.replica {
                item! { w;
                    render_partition_title(ui, w, self.replica, tl!("item-judgement-range-flick"), Some(tl!("item-judgement-range-flick-sub")));
                    render_switch(ui, rr, t, &mut self.flick_range_btn, config.judgement_range_debug.flick);
                }
            }
            self.judgement_range_anchor_btn.render_top(ui, t, 1.);
            ui.dx(-0.035);
        }
        (w, h)
    }
}

/// Replica cards reserve a fixed right-hand control column; original rows retain their layout.
fn render_partition_title<'a>(ui: &mut Ui, w: f32, replica: bool, title: impl Into<Cow<'a, str>>, subtitle: Option<Cow<'a, str>>) -> f32 {
    if !replica {
        return render_title(ui, title, subtitle);
    }
    ui.text(title.into())
        .pos(0.035, if subtitle.is_some() { 0.023 } else { 0.046 })
        .size(0.53)
        .max_width(w - 0.39)
        .draw();
    if let Some(subtitle) = subtitle {
        ui.text(subtitle)
            .pos(0.035, 0.085)
            .size(0.32)
            .color(semi_white(0.7))
            .max_width(w - 0.39)
            .draw();
    }
    ITEM_HEIGHT
}

/// Collapsible groups with progressively indented settings; closed groups never receive input.
struct ReplicaList {
    chart: ChartList,
    debug: DebugList,
    reports: DebugList,
    ranges_btn: DRectButton,
    ranges_open: bool,
    gameplay_btn: DRectButton,
    reports_btn: DRectButton,
    gameplay_open: bool,
    reports_open: bool,
}
impl ReplicaList {
    fn new() -> Self {
        Self {
            chart: ChartList::new(true),
            debug: DebugList::range_settings(),
            reports: DebugList::report_settings(),
            ranges_btn: DRectButton::new(),
            ranges_open: true,
            gameplay_btn: DRectButton::new(),
            reports_btn: DRectButton::new(),
            gameplay_open: true,
            reports_open: true,
        }
    }
    fn top_touch(&mut self, touch: &Touch, t: f32) -> bool {
        (self.gameplay_open && self.chart.top_touch(touch, t)) || (self.ranges_open && self.debug.top_touch(touch, t))
    }
    fn touch(&mut self, touch: &Touch, t: f32) -> Result<Option<bool>> {
        if self.gameplay_btn.touch(touch, t) {
            self.gameplay_open ^= true;
            // Reset popup/hit targets when collapsing, without changing the saved settings.
            self.chart = ChartList::new(true);
            return Ok(Some(false));
        }
        if self.ranges_btn.touch(touch, t) {
            self.ranges_open ^= true;
            self.debug = DebugList::range_settings();
            return Ok(Some(false));
        }
        if self.reports_btn.touch(touch, t) {
            self.reports_open ^= true;
            self.reports = DebugList::report_settings();
            return Ok(Some(false));
        }
        if self.gameplay_open {
            if let Some(changed) = self.chart.touch(touch, t)? {
                return Ok(Some(changed));
            }
        }
        if self.ranges_open {
            if let Some(changed) = self.debug.touch(touch, t)? {
                return Ok(Some(changed));
            }
        }
        if self.reports_open {
            return self.reports.touch(touch, t);
        }
        Ok(None)
    }
    fn update(&mut self, t: f32) -> Result<bool> {
        let chart = self.gameplay_open && self.chart.update(t)?;
        let debug = self.ranges_open && self.debug.update(t)?;
        Ok(chart || debug)
    }
    fn render(&mut self, ui: &mut Ui, r: Rect, t: f32) -> (f32, f32) {
        let w = r.w;
        ui.text(tl!("replica-policy-description"))
            .pos(0.035, 0.015)
            .size(0.42)
            .max_width(w - 0.07)
            .draw();
        ui.dy(0.09);
        let mut h = 0.09;
        for group in 0..3 {
            let (btn, open, label) = if group == 0 {
                (&mut self.gameplay_btn, self.gameplay_open, tl!("replica-gameplay"))
            } else if group == 1 {
                (&mut self.ranges_btn, self.ranges_open, tl!("replica-ranges"))
            } else {
                (&mut self.reports_btn, self.reports_open, tl!("replica-reports"))
            };
            btn.render_text(ui, Rect::new(0., 0., w, 0.105), t, format!("{}  {}", if open { "−" } else { "+" }, label), 0.53, open);
            ui.dy(0.12);
            h += 0.12;
            if open {
                let (_, height) = ui.scope(|ui| {
                    ui.dx(0.035);
                    let child = Rect::new(0., 0., w - 0.035, r.h);
                    if group == 0 {
                        self.chart.render(ui, child, t)
                    } else if group == 1 {
                        self.debug.render(ui, child, t)
                    } else {
                        self.reports.render(ui, child, t)
                    }
                });
                ui.dy(height + 0.025);
                h += height + 0.025;
            }
        }
        ui.text(tl!("replica-chart-editors"))
            .pos(0.025, 0.01)
            .size(0.36)
            .max_width(w - 0.05)
            .draw();
        (w, h + 0.09)
    }
}
