//! Production editor, camera and button hit testing, driven by physical screen pixels.
#![allow(dead_code)]
use phira_chart_play_tests::chart_play;
use std::{cell::RefCell,collections::HashMap};
thread_local!{static STATE:RefCell<HashMap<String,Option<u64>>>=RefCell::new(HashMap::new());}
#[derive(Clone,Copy,Debug,Default)]struct Vec2{x:f32,y:f32}fn vec2(x:f32,y:f32)->Vec2{Vec2{x,y}}
#[derive(Clone,Copy,Debug)]struct Rect{x:f32,y:f32,w:f32,h:f32}
impl Rect{
 fn new(x:f32,y:f32,w:f32,h:f32)->Self{Self{x,y,w,h}}
 fn rounded(self,_:f32)->Self{self}fn center(&self)->Vec2{vec2(self.x+self.w/2.,self.y+self.h/2.)}
 fn contains(&self,p:Vec2)->bool{p.x>=self.x&&p.x<=self.x+self.w&&p.y>=self.y&&p.y<=self.y+self.h}
}
#[derive(Clone,Copy)]struct Color{a:f32}impl Color{fn new(_:f32,_:f32,_:f32,a:f32)->Self{Self{a}}}
const GREEN:Color=Color{a:1.};const RED:Color=GREEN;const WHITE:Color=GREEN;const BLACK:Color=GREEN;
#[derive(Clone,Copy,PartialEq)]enum TouchPhase{Started,Moved,Ended,Cancelled,Stationary}
struct Touch{id:u64,position:Vec2,phase:TouchPhase}
type RenderTarget=u32;
#[derive(Default)]struct Camera2D{zoom:Vec2,viewport:Option<(i32,i32,i32,i32)>,render_target:Option<RenderTarget>}
mod core{#[derive(Clone,Copy)]pub struct Matrix{pub scale:f32}impl Matrix{pub fn new_scaling(scale:f32)->Self{Self{scale}}}}
struct Ui{top:f32,viewport:(i32,i32,i32,i32),s:f32,x:f32,y:f32,touches:Vec<Touch>,buttons:HashMap<String,Rect>,paths:Vec<Rect>}
impl Ui{
 fn new(w:i32,h:i32)->Self{Self{top:h as f32/w as f32,viewport:(0,0,w,h),s:1.,x:0.,y:0.,touches:Vec::new(),buttons:HashMap::new(),paths:Vec::new()}}
 fn dx(&mut self,v:f32){self.x+=v;}fn dy(&mut self,v:f32){self.y+=v;}
 fn scope<T>(&mut self,f:impl FnOnce(&mut Self)->T)->T{let old=(self.s,self.x,self.y);let res=f(self);(self.s,self.x,self.y)=old;res}
 fn with<T>(&mut self,m:core::Matrix,f:impl FnOnce(&mut Self)->T)->T{self.scope(|ui|{ui.s*=m.scale;f(ui)})}
 fn rect_to_global(&self,r:Rect)->Rect{Rect::new(self.x+r.x*self.s,self.y+r.y*self.s,r.w*self.s,r.h*self.s)}
 fn fill_rect(&mut self,r:Rect,_:Color){self.paths.push(self.rect_to_global(r));}
 fn fill_path(&mut self,r:&Rect,c:Color){self.fill_rect(*r,c);}
 fn background(&self)->Color{WHITE}
 fn ensure_touches(&mut self)->&mut Vec<Touch>{&mut self.touches}
 fn text(&mut self,_:impl Into<String>)->Text{Text}
 fn button(&mut self,id:&str,r:Rect,text:impl Into<String>)->bool{self.buttons.insert(id.into(),self.rect_to_global(r));self.raw_button(id,r,text)}
 fn slider(&mut self,_:&str,_:std::ops::Range<f32>,_:f32,_:&mut f32,_:Option<f32>){}
}
struct Text;impl Text{
 fn pos(self,_:f32,_:f32)->Self{self}fn size(self,_:f32)->Self{self}fn no_baseline(self)->Self{self}fn max_width(self,_:f32)->Self{self}
 fn color(self,_:Color)->Self{self}fn anchor(self,_:f32,_:f32)->Self{self}
 fn draw(self){}fn measure(self)->Rect{Rect::new(0.,0.,0.15,0.03)}
}
include!(concat!(env!("OUT_DIR"),"/editor_input.rs"));
include!(concat!(env!("OUT_DIR"),"/chart_editor.rs"));
fn click(e:&mut ChartPlayEditor,s:&mut ChartPlaySettings,ui:&mut Ui,id:&str,time:f64)->Option<ChartPlayEditorAction>{
 e.render(ui,s,time,10.,true);
 let r=ui.buttons[id];let p=r.center();
 let cam=ChartPlayEditor::camera(ui,Some(99));
 assert_eq!(cam.viewport,Some(ui.viewport));assert_eq!(cam.render_target,Some(99));
 // Project the rendered center to pixels; then use the engine's screen touch mapping.
 let px=(p.x*cam.zoom.x+1.)*ui.viewport.2 as f32/2.;
 let py=(1.-p.y*cam.zoom.y)*ui.viewport.3 as f32/2.;
 let point=vec2(px/ui.viewport.2 as f32*2.-1.,(py/ui.viewport.3 as f32*2.-1.)*ui.top);
 assert!((point.x-p.x).abs()<1e-5&&(point.y-p.y).abs()<1e-5);
 assert!(px>=0.&&px<=ui.viewport.2 as f32&&py>=0.&&py<=ui.viewport.3 as f32,"{id} off screen");
 ui.touches=vec![Touch{id:42,position:point,phase:TouchPhase::Started}];e.render(ui,s,time,10.,true);
 ui.touches=vec![Touch{id:42,position:point,phase:TouchPhase::Ended}];e.render(ui,s,time,10.,true)
}
#[test]fn visible_editor_buttons_accept_pixel_taps_on_wide_standard_and_tablet_screens(){
 for (w,h)in [(2400,1080),(1920,1080),(1600,1200),(2800,1080)]{
  STATE.with(|v|v.borrow_mut().clear());
  let mut ui=Ui::new(w,h);let mut s=ChartPlaySettings::default();let mut e=ChartPlayEditor::default();
  assert!(matches!(click(&mut e,&mut s,&mut ui,"flip-preview-play",1.),Some(ChartPlayEditorAction::TogglePause)));
  click(&mut e,&mut s,&mut ui,"flip-mark-start",1.);click(&mut e,&mut s,&mut ui,"flip-mark-end",4.);
  click(&mut e,&mut s,&mut ui,"flip-add",4.);assert_eq!(s.auto_flip_intervals.len(),1);
  click(&mut e,&mut s,&mut ui,"flip-select-0",1.);click(&mut e,&mut s,&mut ui,"flip-mark-end",5.);assert_eq!(s.auto_flip_intervals[0].end,5.);
  click(&mut e,&mut s,&mut ui,"flip-remove-0",5.);assert!(s.auto_flip_intervals.is_empty());
  assert!(matches!(click(&mut e,&mut s,&mut ui,"flip-save",5.),Some(ChartPlayEditorAction::Save)));
  assert!(matches!(click(&mut e,&mut s,&mut ui,"flip-cancel",5.),Some(ChartPlayEditorAction::Cancel)));
  assert!(!ui.buttons.keys().any(|k|k.starts_with("note-conversion")));
 }
}
#[test]fn camera_uses_ui_viewport_with_offsets_and_uniform_pixel_scale(){
 let mut ui=Ui::new(2200,1000);ui.viewport=(100,40,2200,1000);
 let cam=ChartPlayEditor::camera(&ui,None);
 assert_eq!(cam.viewport,Some(ui.viewport));
 assert!((cam.zoom.x*2200.-(-cam.zoom.y)*1000.).abs()<0.001);
}
// Mods uses the same released-inside-button behavior as the existing sidebar controls.
#[derive(Default)]struct DRectButton{rect:Option<Rect>,owner:Option<u64>}
impl DRectButton{
 fn build(&mut self,ui:&mut Ui,_:f32,r:Rect,f:impl FnOnce(&mut Ui,Rect)){self.rect=Some(ui.rect_to_global(r));f(ui,r);}
 fn touch(&mut self,t:&Touch,_:f32)->bool{
  let inside=self.rect.is_some_and(|r|r.contains(t.position));
  match t.phase{TouchPhase::Started=>{if inside{self.owner=Some(t.id);}false},TouchPhase::Ended=>self.owner.take()==Some(t.id)&&inside,TouchPhase::Cancelled=>{self.owner=None;false},_=>false}
 }
}
include!(concat!(env!("OUT_DIR"),"/note_mods.rs"));
#[test]fn mods_choices_are_mutually_exclusive_and_reclick_restores_original(){
 let mut ui=Ui::new(2400,1080);ui.x=0.2;ui.y=-0.4;
 let mut controls=NoteConversionMods::default();let mut current=NoteConversion::Original;
 for (i,want)in [(0,NoteConversion::Tap),(1,NoteConversion::Drag),(2,NoteConversion::Flick),(2,NoteConversion::Original)]{
  controls.render(&mut ui,1.,0.77,current);let p=controls.buttons[i].rect.unwrap().center();
  assert_eq!(controls.touch(&Touch{id:8,position:p,phase:TouchPhase::Started},1.,current),None);
  current=controls.touch(&Touch{id:8,position:p,phase:TouchPhase::Ended},1.1,current).unwrap();assert_eq!(current,want);
 }
}
