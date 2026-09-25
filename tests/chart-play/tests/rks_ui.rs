//! Runs the production RKS editor with recording UI/input adapters (no GPU).
extern crate self as macroquad;
extern crate self as prpr;
extern crate self as inputbox;
#[path="../../../phira/src/custom_rks.rs"] pub mod custom_rks;
#[path="../../../phira/src/page/rks.rs"] pub mod editor;
use std::cell::RefCell;
#[derive(Clone,Copy,Debug,Default)] pub struct Rect {pub x:f32,pub y:f32,pub w:f32,pub h:f32}
impl Rect {pub fn new(x:f32,y:f32,w:f32,h:f32)->Self{Self{x,y,w,h}} pub fn right(&self)->f32{self.x+self.w}pub fn bottom(&self)->f32{self.y+self.h}fn contains(&self,p:Vec2)->bool{p.x>=self.x&&p.x<=self.right()&&p.y>=self.y&&p.y<=self.bottom()}}
#[derive(Clone,Copy)] pub struct Color;impl Color{pub fn from_rgba(_:u8,_:u8,_:u8,_:u8)->Self{Self}}
#[derive(Clone,Copy)] pub struct Vec2{pub x:f32,pub y:f32}
#[derive(Clone,Copy)] pub enum Phase{Started,Ended}
#[derive(Clone,Copy)] pub struct Touch{pub position:Vec2,pub phase:Phase}
pub mod prelude{pub use crate::{Rect,Color,Touch};}
pub mod ext{use super::Rect;pub trait RectExt{fn rounded(&self,r:f32)->Rect;}impl RectExt for Rect{fn rounded(&self,_:f32)->Rect{*self}}}
pub struct InputBox;impl InputBox{pub fn new()->Self{Self}pub fn default_text(self,_:impl Into<String>)->Self{self}}
thread_local!{static REQUEST:RefCell<Option<String>>=const{RefCell::new(None)};static ROOT:RefCell<Option<std::path::PathBuf>>=const{RefCell::new(None)};static DATA:RefCell<Data>=RefCell::new(Data::default());}
pub mod scene{pub fn request_input(id:impl Into<String>,_:crate::InputBox){crate::REQUEST.with(|v|*v.borrow_mut()=Some(id.into()));}pub fn show_message(_:impl Into<String>) {}}
pub mod dir{pub fn root()->anyhow::Result<String>{crate::ROOT.with(|v|Ok(v.borrow().as_ref().unwrap().to_string_lossy().into()))}}
#[derive(Default,Clone)]pub struct Data{pub charts:Vec<Local>,pub local_records:std::collections::HashMap<String,Option<Record>>,pub replica_local_records:std::collections::HashMap<i32,Record>}
#[derive(Clone)]pub struct Local{pub local_path:String,pub record:Option<Record>}
#[derive(Clone)]pub struct Record{pub accuracy:f32}
pub fn get_data()->Data{DATA.with(|v|v.borrow().clone())}
pub mod page{
 pub struct SharedState{pub charts_local:Vec<Chart>}
 pub struct Chart{pub local_path:Option<String>,pub info:Info}
 pub struct Info{pub id:Option<i32>,pub name:String,pub level:String,pub difficulty:f32}
}
pub mod ui{
 use super::*;
 #[derive(Default)]pub struct Ui{pub top:f32,pub buttons:Vec<(String,Rect)>,pub text:Vec<String>}
 impl Ui{pub fn content_rect(&self)->Rect{Rect::new(-0.7,-self.top+0.15,1.67,self.top*2.-0.18)}pub fn fill_path(&mut self,_:&Rect,_:Color){}pub fn text(&mut self,t:impl Into<String>)->Text<'_>{self.text.push(t.into());Text(self)}}
 pub struct Text<'a>(&'a mut Ui);impl Text<'_>{pub fn pos(self,_:f32,_:f32)->Self{self}pub fn size(self,_:f32)->Self{self}pub fn max_width(self,_:f32)->Self{self}pub fn draw(self){let _=&self.0;}}
 #[derive(Default)]pub struct RectButton{r:Option<Rect>,down:bool}
 impl RectButton{pub fn new()->Self{Self::default()}pub fn cancel(&mut self){self.down=false;}pub fn set(&mut self,_:&mut Ui,r:Rect){self.r=Some(r);}pub fn contains(&self,p:Vec2)->bool{self.r.is_some_and(|r|r.contains(p))}pub fn touching(&self)->bool{self.down}}
 pub struct DRectButton{pub inner:RectButton}impl DRectButton{pub fn new()->Self{Self{inner:RectButton::new()}}pub fn render_text(&mut self,u:&mut Ui,r:Rect,_:f32,t:impl Into<String>,_:f32,_:bool){self.inner.set(u,r);u.buttons.push((t.into(),r));}pub fn touch(&mut self,t:&Touch,_:f32)->bool{let inside=self.inner.contains(t.position);match t.phase{Phase::Started=>{self.inner.down=inside;false}Phase::Ended=>{let ret=self.inner.down&&inside;self.inner.down=false;ret}}}}
 pub fn button_hit(){}
}
fn setup()->(tempfile::TempDir,editor::RksPanel,page::SharedState){
 let root=tempfile::tempdir().unwrap();ROOT.with(|v|*v.borrow_mut()=Some(root.path().into()));
 DATA.with(|v|*v.borrow_mut()=Data{charts:vec![Local{local_path:"local/a".into(),record:Some(Record{accuracy:0.981234})},Local{local_path:"local/b".into(),record:None}],local_records:Default::default(),replica_local_records:Default::default()});
 let state=page::SharedState{charts_local:vec![page::Chart{local_path:Some("local/a".into()),info:page::Info{id:None,name:"A".into(),level:"IN Lv.16".into(),difficulty:16.3}},page::Chart{local_path:Some("local/b".into()),info:page::Info{id:None,name:"B".into(),level:"HOT.15 Lv.15".into(),difficulty:15.}}]};
 let mut panel=editor::RksPanel::new().unwrap();panel.refresh(&state);(root,panel,state)
}
fn render(p:&mut editor::RksPanel)->ui::Ui{let mut u=ui::Ui{top:926./2048.,..Default::default()};p.begin_render();if p.editing{p.render_header(&mut u,1.);p.render_overlay(&mut u,1.);}else{p.render_entry(&mut u,1.,Rect::new(-0.2,-0.4,0.3,0.09));}u}
fn click(p:&mut editor::RksPanel,label:&str){let u=render(p);let (_,r)=u.buttons.iter().find(|(s,_)|s.contains(label)).unwrap_or_else(||panic!("missing {label}: {:?}",u.buttons));let position=Vec2{x:r.x+r.w/2.,y:r.y+r.h/2.};assert!(p.touch(&Touch{position,phase:Phase::Started},1.).unwrap());assert!(p.touch(&Touch{position,phase:Phase::Ended},1.01).unwrap());}
fn input(p:&mut editor::RksPanel,text:&str)->anyhow::Result<()>{let id=REQUEST.with(|v|v.borrow_mut().take().unwrap());p.input(&id,text.into())}
#[test]
fn editor_sources_fields_toggle_back_and_persistence(){
 let (_root,mut p,state)=setup();click(&mut p,"RKS:");assert!(p.editing);
 p.action(editor::RksAction::Edit("local/a".into())).unwrap();
 click(&mut p,"自定义"); // first custom is difficulty
 click(&mut p,"定数：");input(&mut p,"16.87").unwrap();assert_eq!(p.resolved["local/a"].difficulty,Some(16.9));
 assert!(p.resolved["local/a"].difficulty_changed);
 p.action(editor::RksAction::Toggle("local/a".into())).unwrap();assert!(!p.resolved["local/a"].included);
 assert!(p.back());assert!(p.editing);assert!(p.back());assert!(!p.editing);
 let mut q=editor::RksPanel::new().unwrap();q.refresh(&state);assert_eq!(q.resolved["local/a"].difficulty,Some(16.9));assert!(!q.resolved["local/a"].included);
}
#[test]
fn pending_numeric_edit_stays_bound_to_original_chart(){
 let (_root,mut p,_)=setup();click(&mut p,"RKS:");p.action(editor::RksAction::Edit("local/a".into())).unwrap();click(&mut p,"自定义");click(&mut p,"定数：");
 p.action(editor::RksAction::Edit("local/b".into())).unwrap();input(&mut p,"17.1").unwrap();assert_eq!(p.resolved["local/a"].difficulty,Some(17.1));assert_eq!(p.resolved["local/b"].difficulty,Some(15.));
}
#[test]
fn extra_entry_input_validation_rules_and_deletion(){
 let (root,mut p,_)=setup();click(&mut p,"RKS:");click(&mut p,"额外条目");click(&mut p,"添加条目");click(&mut p,"歌曲名：");input(&mut p,"Manual Test").unwrap();click(&mut p,"定数：");input(&mut p,"15.8").unwrap();click(&mut p,"准确率：");assert!(input(&mut p,"101").is_err());
 click(&mut p,"准确率：");input(&mut p,"99.126").unwrap();
 let s=custom_rks::Settings::load(&root.path().join("custom-rks.json")).unwrap();assert_eq!(s.extras[0].accuracy,99.13);assert_eq!(s.extras[0].name,"Manual Test");
 click(&mut p,"删除此条目");assert!(custom_rks::Settings::load(&root.path().join("custom-rks.json")).unwrap().extras.is_empty());
 click(&mut p,"计算规则");click(&mut p,"最佳成绩数量");input(&mut p,"19").unwrap();click(&mut p,"AP 成绩数量");input(&mut p,"1").unwrap();
 let s=custom_rks::Settings::load(&root.path().join("custom-rks.json")).unwrap();assert_eq!((s.best_count,s.ap_count),(19,1));
}
#[test]
fn production_buttons_fit_screenshot_and_do_not_overlap(){
 let (_root,mut p,_)=setup();click(&mut p,"RKS:");p.action(editor::RksAction::Edit("local/a".into())).unwrap();
 let u=render(&mut p);for (label,r) in &u.buttons{assert!(r.x>=-1.&&r.right()<=1.&&r.y>=-u.top&&r.bottom()<=u.top,"out of screen: {label}: {r:?}");}
 for (i,(a,ra)) in u.buttons.iter().enumerate(){for(b,rb)in u.buttons.iter().skip(i+1){let overlap=ra.x<rb.right()-1e-5&&ra.right()>rb.x+1e-5&&ra.y<rb.bottom()-1e-5&&ra.bottom()>rb.y+1e-5;assert!(!overlap,"{a} overlaps {b}");}}
}
#[test]
fn bulk_selection_keeps_sources_and_separates_local_from_extra_entries() {
 let (root,_,state)=setup();
 let path=root.path().join("custom-rks.json");
 let mut s=custom_rks::Settings::default();
 s.charts.insert("local/a".into(),custom_rks::ChartSettings {
  included:false,difficulty_source:custom_rks::DifficultySource::Custom,custom_difficulty:Some(17.1),
  accuracy_source:custom_rks::AccuracySource::Custom,custom_accuracy:Some(99.12),
 });
 s.extras=vec![custom_rks::ExtraEntry{id:1,name:"Extra".into(),difficulty:15.6,accuracy:100.,included:true}];
 s.save(&path).unwrap();
 let mut p=editor::RksPanel::new().unwrap();p.refresh(&state);click(&mut p,"RKS:");
 click(&mut p,"全取消");assert!(p.resolved.values().all(|r|!r.included));
 assert!(custom_rks::Settings::load(&path).unwrap().extras[0].included);
 click(&mut p,"全选");assert!(p.resolved.values().all(|r|r.included));
 assert_eq!(p.resolved["local/a"].difficulty,Some(17.1));assert_eq!(p.resolved["local/a"].accuracy,Some(99.12));
 click(&mut p,"额外条目");click(&mut p,"全取消");
 let saved=custom_rks::Settings::load(&path).unwrap();assert!(!saved.extras[0].included);assert!(saved.chart("local/a").included);
 click(&mut p,"全选");assert!(custom_rks::Settings::load(&path).unwrap().extras[0].included);
 let mut q=editor::RksPanel::new().unwrap();q.refresh(&state);assert!(q.resolved.values().all(|r|r.included));
 assert_eq!(q.resolved["local/a"].accuracy,Some(99.12));assert!(q.resolved["local/a"].accuracy_custom);
 click(&mut p,"计算规则");assert!(!render(&mut p).buttons.iter().any(|(s,_)|s=="全选"||s=="全取消"));
}
#[test]
fn custom_accuracy_colour_follows_source_even_when_value_matches_record() {
 let input=custom_rks::LocalInput{path:"a".into(),name:"A".into(),level:"IN Lv.16".into(),difficulty:16.,ai_difficulty:None,accuracy:Some(99.12)};
 let mut settings=custom_rks::ChartSettings::default();settings.custom_accuracy=Some(99.12);
 let record=custom_rks::resolve(&input,&settings);assert!(!record.accuracy_custom);
 settings.accuracy_source=custom_rks::AccuracySource::Custom;
 let custom=custom_rks::resolve(&input,&settings);assert!(custom.accuracy_custom);assert_eq!(custom.accuracy,record.accuracy);
 settings.accuracy_source=custom_rks::AccuracySource::Record;assert!(!custom_rks::resolve(&input,&settings).accuracy_custom);
}
#[test]
fn rks_follows_maximum_record_without_changing_custom_overrides() {
 let (_root,mut p,mut state)=setup();state.charts_local[0].info.id=Some(7);
 DATA.with(|v|{let mut d=v.borrow_mut();d.replica_local_records.insert(7,Record{accuracy:0.9912});});
 p.refresh(&state);assert!((p.resolved["local/a"].accuracy.unwrap()-99.12).abs()<1e-4);
 DATA.with(|v|v.borrow_mut().charts[0].record=Some(Record{accuracy:0.9988}));
 p.refresh(&state);assert!((p.resolved["local/a"].accuracy.unwrap()-99.88).abs()<1e-4);
 assert!(!p.resolved["local/a"].accuracy_custom);
 assert_eq!(custom_rks::highest_record_accuracy(Some(98.),Some(99.)),Some(99.));
 assert_eq!(custom_rks::highest_record_accuracy(Some(100.),Some(99.)),Some(100.));
 assert_eq!(custom_rks::highest_record_accuracy(None,Some(97.)),Some(97.));
 assert_eq!(custom_rks::highest_record_accuracy(Some(f64::NAN),Some(99.9999)),Some(99.9999));
}

pub mod ai_service {
    pub fn revision()->u64 { 0 }
    pub fn value(_: &str)->Option<f64> { None }
    pub fn status(_: &str)->String { "等待预测".into() }
    pub fn catalog(_:impl IntoIterator<Item=String>) {}
    pub fn priorities(_:impl IntoIterator<Item=String>) {}
}
#[test]
fn ai_source_is_selectable_persists_and_does_not_invent_pending_values(){
 let (_root,mut p,state)=setup();click(&mut p,"RKS:");
 p.action(editor::RksAction::Edit("local/a".into())).unwrap();click(&mut p,"AI 计算");
 assert_eq!(p.resolved["local/a"].difficulty,None);
 let mut restored=editor::RksPanel::new().unwrap();restored.refresh(&state);
 assert_eq!(restored.resolved["local/a"].difficulty,None);
 click(&mut p,"跟随文件");assert_eq!(p.resolved["local/a"].difficulty,Some(16.3));
}
