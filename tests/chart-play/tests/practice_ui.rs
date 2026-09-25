//! Real slider, button and touch-consumption code with recording graphics/input adapters.
#![allow(dead_code)]
use std::{cell::RefCell,collections::HashMap,ops::Range};
use phira_chart_play_tests::practice_speed;
thread_local! {
    static STATE:RefCell<HashMap<String,Option<u64>>>=RefCell::new(HashMap::new());
    static REQUEST:RefCell<Option<(String,String)>>=const{RefCell::new(None)};
}
#[derive(Clone,Copy,Debug)]struct Vec2{x:f32,y:f32}
#[derive(Clone,Copy,Debug)]struct Rect{x:f32,y:f32,w:f32,h:f32}
impl Rect {
    fn new(x:f32,y:f32,w:f32,h:f32)->Self{Self{x,y,w,h}}
    fn feather(self,d:f32)->Self{Self::new(self.x-d,self.y-d,self.w+2.*d,self.h+2.*d)}
    fn right(self)->f32{self.x+self.w}
    fn rounded(self,_:f32)->Self{self}
    fn center(self)->Vec2{Vec2{x:self.x+self.w/2.,y:self.y+self.h/2.}}
    fn contains(self,p:Vec2)->bool{p.x>=self.x&&p.x<=self.x+self.w&&p.y>=self.y&&p.y<=self.y+self.h}
}
#[derive(Clone,Copy)]struct Color{a:f32}
const WHITE:Color=Color{a:1.};
#[derive(Clone,Copy,PartialEq)]enum TouchPhase{Started,Moved,Ended,Cancelled,Stationary}
struct Touch{id:u64,position:Vec2,phase:TouchPhase}
struct InputBox{text:String}
impl InputBox{fn new()->Self{Self{text:String::new()}}fn default_text(mut self,text:impl Into<String>)->Self{self.text=text.into();self}}
fn request_input(id:&str,b:InputBox){REQUEST.with(|r|*r.borrow_mut()=Some((id.into(),b.text)));}
#[derive(Default)]struct Ui{touches:Option<Vec<Touch>>,circles:Vec<(f32,f32,f32)>,texts:Vec<String>,paths:Vec<Rect>}
impl Ui {
    fn text(&mut self,s:impl Into<String>)->Text<'_>{let text=s.into();let count=text.chars().count();self.texts.push(text);Text{ui:self,count,size:0.4,x:0.,y:0.,width:10.}}
    fn ensure_touches(&mut self)->&mut Vec<Touch>{self.touches.get_or_insert_with(Vec::new)}
    fn fill_rect(&mut self,_:Rect,_:Color){}
    fn fill_path(&mut self,r:&Rect,_:Color){self.paths.push(*r);}
    fn fill_circle(&mut self,x:f32,y:f32,r:f32,_:Color){self.circles.push((x,y,r));}
    fn rect_to_global(&self,r:Rect)->Rect{r}
    fn to_local(&self,p:(f32,f32))->(f32,f32){p}
    fn accent(&self)->Color{WHITE}
    fn background(&self)->Color{WHITE}
}
struct Text<'a>{ui:&'a mut Ui,count:usize,size:f32,x:f32,y:f32,width:f32}
impl Text<'_>{
    fn size(mut self,s:f32)->Self{self.size=s;self}fn pos(mut self,x:f32,y:f32)->Self{self.x=x;self.y=y;self}
    fn anchor(self,_:f32,_:f32)->Self{self}fn max_width(mut self,w:f32)->Self{assert!(w>0.);self.width=w;self}
    fn color(self,_:Color)->Self{self}fn no_baseline(self)->Self{self}
    fn draw(self)->Rect{Rect::new(self.x,self.y,(self.count as f32*self.size*0.06).min(self.width),self.size*0.1)}
}
include!(concat!(env!("OUT_DIR"),"/practice_slider.rs"));
fn frame(value:&mut f32,id:&str,x:f32,y:f32,phase:TouchPhase)->Ui {
    let mut ui=Ui{touches:Some(vec![Touch{id:7,position:Vec2{x,y},phase}]),..Default::default()};
    ui.practice_speed_slider(id,"Speed",0.05..10.,0.05,value,Some(0.46));ui
}
#[test]
fn dragging_visits_both_extremes_and_exact_midpoint(){
    let mut value=1.;
    let ui=frame(&mut value,"speed",0.23,0.115,TouchPhase::Started);
    assert_eq!(ui.circles[0].0,0.23);assert!(ui.texts.iter().any(|s|s=="1.00"));
    frame(&mut value,"speed",0.,0.115,TouchPhase::Moved);assert_eq!(value,0.05);
    frame(&mut value,"speed",0.23,0.115,TouchPhase::Moved);assert_eq!(value,1.);
    frame(&mut value,"speed",0.46,0.115,TouchPhase::Ended);assert_eq!(value,10.);
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],None));
}
#[test]
fn numeric_button_follows_label_and_remains_clickable(){
    let mut value=1.;
    let mut ui=Ui::default();
    ui.practice_speed_slider("exercise_speed","Speed",0.05..10.,0.05,&mut value,Some(0.46));
    let number=ui.paths[0];assert!(number.right()<0.4);assert!(number.h>=0.07);
    let p=number.center();
    frame(&mut value,"exercise_speed",p.x,p.y,TouchPhase::Started);
    frame(&mut value,"exercise_speed",p.x,p.y,TouchPhase::Ended);
    REQUEST.with(|r|assert_eq!(r.borrow().as_ref().unwrap(),&("exercise_speed".into(),"1.00".into())));
    assert_eq!(value,1.);
    STATE.with(|s|assert_eq!(s.borrow()["exercise_speed:drag"],None));
}
#[test]
fn cancellation_does_not_commit_position_and_larger_buttons_remain_linear(){
    let mut value=1.;
    frame(&mut value,"speed",0.23,0.115,TouchPhase::Started);
    let ui=frame(&mut value,"speed",0.46,0.115,TouchPhase::Cancelled);assert_eq!(value,1.);
    let minus=ui.paths[1];let plus=ui.paths[2];
    assert_eq!(minus.w,0.08);assert_eq!(plus.w,0.08);assert!(plus.x-minus.right()>0.024);
    assert!(plus.right()+0.3<=1.); // actual practice panel starts at x=0.3
    for (button,expected) in [(plus,1.05),(minus,1.)] {
        let p=button.center();frame(&mut value,"speed",p.x,p.y,TouchPhase::Started);
        frame(&mut value,"speed",p.x,p.y,TouchPhase::Ended);assert!((value-expected).abs()<1e-6);
    }
}
#[test]
fn enlarged_hit_area_captures_and_empty_frames_do_not_release(){
    let mut value=1.;
    frame(&mut value,"speed",0.23,0.155,TouchPhase::Started); // 0.04 off the rail
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],Some(7)));
    let mut ui=Ui::default();ui.practice_speed_slider("speed","Speed",0.05..10.,0.05,&mut value,Some(0.46));
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],Some(7)));
    frame(&mut value,"speed",0.46,-0.8,TouchPhase::Moved);assert_eq!(value,10.);
    frame(&mut value,"speed",0.,0.8,TouchPhase::Ended);assert_eq!(value,0.05);
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],None));
}
#[test]
fn owner_crossing_number_or_other_controls_never_clicks_them(){
    let mut value=1.;
    let ui=frame(&mut value,"speed",0.23,0.115,TouchPhase::Started);
    let p=ui.paths[0].center();
    frame(&mut value,"speed",p.x,p.y,TouchPhase::Moved);
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],Some(7)));
    let mut ui=frame(&mut value,"speed",p.x,p.y,TouchPhase::Ended);
    REQUEST.with(|r|assert!(r.borrow().is_none()));
    assert!(ui.touches.as_ref().unwrap().is_empty());
    let mut other=1.;ui.practice_speed_slider("other","Flow",0.1..20.,0.05,&mut other,Some(0.46));assert_eq!(other,1.);
}
#[test]
fn unrelated_finger_release_does_not_end_drag_and_scene_cleanup_does(){
    let mut value=1.;frame(&mut value,"exercise_speed",0.23,0.115,TouchPhase::Started);
    let mut ui=Ui{touches:Some(vec![Touch{id:8,position:Vec2{x:0.,y:0.8},phase:TouchPhase::Ended}]),..Default::default()};
    ui.practice_speed_slider("exercise_speed","Speed",0.05..10.,0.05,&mut value,Some(0.46));
    STATE.with(|s|assert_eq!(s.borrow()["exercise_speed:drag"],Some(7)));
    Ui::clear_practice_speed_drag();
    STATE.with(|s|assert!(!s.borrow().contains_key("exercise_speed:drag")));
    frame(&mut value,"exercise_speed",0.46,0.115,TouchPhase::Moved);assert_eq!(value,1.);
}
