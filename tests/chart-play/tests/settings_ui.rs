//! Production group layout/input tests with recording adapters; no GPU, font, popup or Android test.
#![allow(dead_code)]
use anyhow::Result;
use phira_chart_play_tests::config::*;
use std::{borrow::Cow, cell::UnsafeCell};
const ITEM_HEIGHT: f32 = 0.15;
const INTERACT_WIDTH: f32 = 0.26;
macro_rules! tl { ($key:literal) => { Cow::Borrowed($key) }; }
macro_rules! ttl { ($key:literal) => { Cow::Borrowed($key) }; }
struct Data { config: Config }
thread_local! { static DATA: UnsafeCell<Data> = UnsafeCell::new(Data { config: Config::default() }); }
fn get_data() -> &'static Data { DATA.with(|d| unsafe { &*d.get() }) }
fn get_data_mut() -> &'static mut Data { DATA.with(|d| unsafe { &mut *d.get() }) }
#[derive(Clone, Copy, Debug)]
struct Rect { x:f32, y:f32, w:f32, h:f32 }
impl Rect {
    fn new(x:f32,y:f32,w:f32,h:f32)->Self {Self{x,y,w,h}}
    fn rounded(self,_:f32)->Self {self}
    fn contains(self,t:&Touch)->bool {t.x>=self.x&&t.x<=self.x+self.w&&t.y>=self.y&&t.y<=self.y+self.h}
}
struct Color;
impl Color {fn from_rgba(_:u8,_:u8,_:u8,_:u8)->Self {Self}}
fn semi_white(_:f32)->Color {Color}
struct Touch { x:f32, y:f32, down:bool }
#[derive(Default)]
struct Ui { x:f32,y:f32,buttons:Vec<(String,Rect)>,text:Vec<(String,f32,f32)>,cards:Vec<Rect> }
impl Ui {
    fn rect(&self,mut r:Rect)->Rect {r.x+=self.x;r.y+=self.y;r}
    fn dx(&mut self,x:f32){self.x+=x;}
    fn dy(&mut self,y:f32){self.y+=y;}
    fn fill_path(&mut self,r:&Rect,_:Color){self.cards.push(self.rect(*r));}
    fn text(&mut self,text:impl Into<String>)->Text<'_>{Text{ui:self,text:text.into(),x:0.,y:0.}}
    fn scope<T>(&mut self,f:impl FnOnce(&mut Self)->T)->T {let old=(self.x,self.y);let out=f(self);self.x=old.0;self.y=old.1;out}
}
struct Text<'a> {ui:&'a mut Ui,text:String,x:f32,y:f32}
impl Text<'_> {
    fn pos(mut self,x:f32,y:f32)->Self{self.x=x;self.y=y;self}
    fn size(self,_:f32)->Self{self}
    fn max_width(self,w:f32)->Self{assert!(w>0.);self}
    fn color(self,_:Color)->Self{self}
    fn draw(self){self.ui.text.push((self.text,self.ui.x+self.x,self.ui.y+self.y));}
}
struct DRectButton {r:Option<Rect>,down:bool}
impl DRectButton {
    fn new()->Self{Self{r:None,down:false}}
    fn render_text(&mut self,ui:&mut Ui,r:Rect,_:f32,label:impl Into<String>,_:f32,_:bool){let r=ui.rect(r);self.r=Some(r);ui.buttons.push((label.into(),r));}
    fn touch(&mut self,t:&Touch,_:f32)->bool {
        let inside=self.r.is_some_and(|r|r.contains(t));
        if t.down {self.down=inside;false} else {let hit=self.down&&inside;self.down=false;hit}
    }
}
struct ChooseButton {btn:DRectButton,options:Vec<String>,selected:usize,dirty:bool}
impl ChooseButton {
    fn new()->Self{Self{btn:DRectButton::new(),options:vec![],selected:0,dirty:false}}
    fn with_options(mut self,v:Vec<String>)->Self{self.options=v;self}
    fn with_selected(mut self,v:usize)->Self{self.selected=v;self}
    fn selected(&self)->usize{self.selected}
    fn changed(&mut self)->bool{std::mem::take(&mut self.dirty)}
    fn update(&mut self,_:f32){}
    fn top_touch(&mut self,_:&Touch,_:f32)->bool{false}
    fn touch(&mut self,t:&Touch,time:f32)->bool{if self.btn.touch(t,time){self.selected=(self.selected+1)%self.options.len();self.dirty=true;true}else{false}}
    fn render(&mut self,ui:&mut Ui,r:Rect,t:f32){self.btn.render_text(ui,r,t,self.options[self.selected].clone(),0.5,false);}
    fn render_top(&mut self,_:&mut Ui,_:f32,_:f32){}
}
struct Slider {btn:DRectButton}
impl Slider {
    fn new(_:std::ops::Range<f32>,_:f32)->Self{Self{btn:DRectButton::new()}}
    fn touch(&mut self,_:&Touch,_:f32,_:&mut f32)->Option<bool>{None}
    fn render(&mut self,u:&mut Ui,r:Rect,t:f32,_:f32,text:String){self.btn.render_text(u,r,t,text,0.5,false);}
}
fn render_title<'a>(ui:&mut Ui,title:impl Into<Cow<'a,str>>,sub:Option<Cow<'a,str>>)->f32{
    ui.text(title.into()).draw();if let Some(sub)=sub{ui.text(sub).draw();}ITEM_HEIGHT
}
include!(concat!(env!("OUT_DIR"), "/settings_groups.rs"));
fn setup()->ReplicaList{get_data_mut().config=Config::default();ReplicaList::new()}
fn render(p:&mut ReplicaList,w:f32)->Ui{let mut ui=Ui::default();p.render(&mut ui,Rect::new(0.,0.,w,1.),1.);ui}
fn click(p:&mut ReplicaList,r:Rect){for down in [true,false]{p.touch(&Touch{x:r.x+r.w/2.,y:r.y+r.h/2.,down},1.).unwrap();}p.update(1.).unwrap();}
fn find(ui:&Ui,label:&str)->Rect {ui.buttons.iter().find(|(s,_)|s.contains(label)).unwrap().1}
#[test]
fn single_choice_updates_every_note_and_ap_fc_stays_in_chart_tab(){
    let mut p=setup();let ui=render(&mut p,1.6);
    assert_eq!(ui.buttons.iter().filter(|(s,_)|s.contains("judgement-mode-")).count(),1);
    assert!(!ui.text.iter().any(|(s,_,_)|s=="item-speed"||s=="item-ap-fc-indicator"));
    click(&mut p,find(&ui,"judgement-mode-phira"));
    assert_eq!(get_data().config.judgement_mode,JudgementMode::PhigrosReplica);
    assert_eq!(get_data().config.flick_judgement_mode(),JudgementMode::PhigrosReplica);
    let ui=render(&mut p,1.6);assert!(ui.text.iter().any(|(s,_,_)|s=="item-phigros-strict-judgement"));
    click(&mut p,find(&ui,"judgement-mode-phigros"));
    assert_eq!(get_data().config.flick_judgement_mode(),JudgementMode::Phira);
    let mut chart=ChartList::new(false);let mut ui=Ui::default();chart.render(&mut ui,Rect::new(0.,0.,1.6,1.),1.);
    assert!(ui.text.iter().any(|(s,_,_)|s=="item-ap-fc-indicator"));
}
#[test]
fn collapsed_groups_ignore_old_hit_rectangles_without_changing_values(){
    let mut p=setup();get_data_mut().config.judgement_range_debug.enabled=true;
    let ui=render(&mut p,1.6);let old=p.debug.tap_layer_btns[0].r.unwrap();
    click(&mut p,find(&ui,"replica-ranges"));assert!(!p.ranges_open);
    let before=serde_json::to_value(&get_data().config).unwrap();click(&mut p,old);
    assert_eq!(serde_json::to_value(&get_data().config).unwrap(),before);
    let ui=render(&mut p,1.6);assert!(!ui.buttons.iter().any(|(s,_)|s=="Perfect"));
    click(&mut p,find(&ui,"replica-ranges"));assert!(p.ranges_open);
    assert!(get_data().config.judgement_range_debug.tap.perfect);
}
#[test]
fn expanded_children_are_indented_and_controls_do_not_overlap(){
    let mut p=setup();get_data_mut().config.judgement_range_debug.enabled=true;
    get_data_mut().config.judgement_mode=JudgementMode::PhigrosReplica;
    for w in [1.2,1.6] {
        let ui=render(&mut p,w);
        let x=|key:&str|ui.text.iter().find(|(s,_,_)|s==key).unwrap().1;
        assert!(x("item-judgement-range-anchor")>x("item-judgement-range-debug"));
        assert!(find(&ui,"Perfect").x>x("item-judgement-range-tap"));
        assert!(x("item-phigros-strict-judgement")>x("item-judgement-mode"));
        for r in &ui.cards {assert!(r.x>=-1e-5&&r.x+r.w<=w+1e-5,"card overflow: {r:?}");}
        for (i,(name,a)) in ui.buttons.iter().enumerate() {
            assert!(a.x>=-1e-5&&a.x+a.w<=w+1e-5,"button overflow: {name}");
            for (name2,b) in ui.buttons.iter().skip(i+1) {
                let overlap=a.x<b.x+b.w-1e-5&&a.x+a.w>b.x+1e-5&&a.y<b.y+b.h-1e-5&&a.y+a.h>b.y+1e-5;
                assert!(!overlap,"{name} overlaps {name2}");
            }
        }
        for word in ["Perfect","Good","Bad","Miss","Tail"]{assert!(ui.buttons.iter().any(|(s,_)|s==word));}
    }
}
#[test]
fn disabled_range_and_note_groups_ignore_hidden_children(){
    let mut p=setup();get_data_mut().config.judgement_range_debug.enabled=true;render(&mut p,1.6);
    let layer=p.debug.tap_layer_btns[0].r.unwrap();let tap=p.debug.tap_range_btn.r.unwrap();
    click(&mut p,tap);assert!(!get_data().config.judgement_range_debug.tap.enabled);
    click(&mut p,layer);assert!(get_data().config.judgement_range_debug.tap.perfect);
    let main=p.debug.judgement_range_btn.r.unwrap();let hold=p.debug.hold_range_btn.r.unwrap();
    click(&mut p,main);assert!(!get_data().config.judgement_range_debug.enabled);
    click(&mut p,hold);assert!(get_data().config.judgement_range_debug.hold.enabled);
}

#[test]
fn visualization_has_its_own_group_above_reports_and_collapses_independently(){
    let mut p=setup();let ui=render(&mut p,1.6);
    assert!(find(&ui,"replica-gameplay").y<find(&ui,"replica-ranges").y);
    assert!(find(&ui,"replica-ranges").y<find(&ui,"replica-reports").y);
    assert_eq!(ui.text.iter().filter(|(s,_,_)|s=="replica-policy-description").count(),1);
    click(&mut p,find(&ui,"replica-ranges"));
    let ui=render(&mut p,1.6);
    assert!(!ui.text.iter().any(|(s,_,_)|s=="item-judgement-range-debug"));
    assert!(ui.text.iter().any(|(s,_,_)|s=="item-play-report"));
    click(&mut p,find(&ui,"replica-reports"));
    let ui=render(&mut p,1.6);
    click(&mut p,find(&ui,"replica-ranges"));
    let ui=render(&mut p,1.6);
    assert!(!ui.text.iter().any(|(s,_,_)|s=="item-play-report"));
}
