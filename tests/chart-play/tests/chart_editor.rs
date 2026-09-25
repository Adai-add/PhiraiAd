//! Execute the production editor with deterministic button and drawing adapters.
#![allow(dead_code)]
use phira_chart_play_tests::chart_play;
use std::collections::HashSet;
mod core {pub struct Matrix;impl Matrix{pub fn new_scaling(_:f32)->Self{Self}}}
#[derive(Clone,Copy)]struct Color;impl Color{fn new(_:f32,_:f32,_:f32,_:f32)->Self{Self}}
const GREEN:Color=Color;const RED:Color=Color;
struct Rect{h:f32}impl Rect{fn new(_:f32,_:f32,_:f32,h:f32)->Self{Self{h}}}
#[derive(Default)]struct Camera2D{render_target:Option<RenderTarget>}
type RenderTarget=u32;
struct Ui{top:f32,clicks:HashSet<String>}
impl Ui{
 fn new(ids:&[&str])->Self{Self{top:0.5625,clicks:ids.iter().map(|s|s.to_string()).collect()}}
 fn camera(&self)->Camera2D{Camera2D::default()}
 fn dx(&mut self,_:f32){}fn dy(&mut self,_:f32){}
 fn scope<T>(&mut self,f:impl FnOnce(&mut Self)->T)->T{f(self)}
 fn with<T>(&mut self,_:core::Matrix,f:impl FnOnce(&mut Self)->T)->T{f(self)}
 fn fill_rect(&mut self,_:Rect,_:Color){}
 fn text(&mut self,_:impl Into<String>)->Text{Text}
 fn button(&mut self,id:&str,_:Rect,_:impl Into<String>)->bool{self.clicks.remove(id)}
 fn slider(&mut self,_:&str,_:std::ops::Range<f32>,_:f32,_:&mut f32,_:Option<f32>){}
}
struct Text;impl Text{
 fn pos(self,_:f32,_:f32)->Self{self}fn size(self,_:f32)->Self{self}fn no_baseline(self)->Self{self}fn max_width(self,_:f32)->Self{self}
 fn draw(self){}fn measure(self)->Rect{Rect{h:0.03}}
}
include!(concat!(env!("OUT_DIR"),"/chart_editor.rs"));
#[test]fn selected_interval_changes_without_update_button(){
 let mut s=ChartPlaySettings::default();s.add(2.,4.);let mut e=ChartPlayEditor::default();
 e.render(&mut Ui::new(&["flip-select-0"]),&mut s,0.,10.,true);
 e.render(&mut Ui::new(&["flip-mark-end"]),&mut s,5.,10.,true);
 assert_eq!(s.auto_flip_intervals[0].end,5.);assert!(s.flipped_at(4.5));
 e.render(&mut Ui::new(&["flip-fine-0"]),&mut s,5.,10.,true);
 assert!((s.auto_flip_intervals[0].start-1.99).abs()<1e-5);
 e.render(&mut Ui::new(&["flip-remove-0"]),&mut s,5.,10.,true);assert!(s.auto_flip_intervals.is_empty());
}
#[test]fn editor_preserves_conversion_from_mods_and_saves_only_interval_edits(){
 let mut s=ChartPlaySettings::default();s.note_conversion=chart_play::NoteConversion::Flick;
 let mut e=ChartPlayEditor::default();
 e.render(&mut Ui::new(&["flip-mark-start"]),&mut s,1.,10.,true);
 e.render(&mut Ui::new(&["flip-mark-end"]),&mut s,3.,10.,true);
 e.render(&mut Ui::new(&["flip-add"]),&mut s,3.,10.,true);
 assert!(matches!(e.render(&mut Ui::new(&["flip-save"]),&mut s,3.,10.,true),Some(ChartPlayEditorAction::Save)));
 assert_eq!(s.note_conversion,chart_play::NoteConversion::Flick);assert!(s.flipped_at(2.));
}
#[test]fn merging_selected_range_keeps_merged_bounds_for_next_edit(){
 let mut s=ChartPlaySettings::default();s.add(1.,2.);s.add(2.5,4.);let mut e=ChartPlayEditor::default();
 e.render(&mut Ui::new(&["flip-select-0"]),&mut s,1.,10.,true);
 e.render(&mut Ui::new(&["flip-mark-end"]),&mut s,3.,10.,true);
 assert_eq!(s.auto_flip_intervals.len(),1);assert_eq!(e.end,4.);
 e.render(&mut Ui::new(&["flip-fine-0"]),&mut s,3.,10.,true);assert_eq!(s.auto_flip_intervals[0].end,4.);
}
