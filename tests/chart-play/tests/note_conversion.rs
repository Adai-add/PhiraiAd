use phira_chart_play_tests::chart_play;
mod judge {
 #[derive(Clone,Debug,PartialEq)]pub enum HitSound{Tap,Drag,Flick,Custom}
 impl HitSound{pub fn default_from_kind(k:&crate::core::NoteKind)->Self{match k{crate::core::NoteKind::Click|crate::core::NoteKind::Hold{..}=>Self::Tap,crate::core::NoteKind::Drag=>Self::Drag,crate::core::NoteKind::Flick=>Self::Flick}}}
}
mod core {
 #[derive(Clone,Debug,PartialEq)]pub enum NoteKind{Click,Hold{end_time:f64,end_height:f64},Drag,Flick}
 pub struct Anim;impl Anim{fn now(&self)->f32{0.}}
 pub struct Object{pub translation:(Anim,Anim)}
 pub struct Note{pub kind:NoteKind,pub hitsound:crate::judge::HitSound,pub time:f64,pub height:f64,pub fake:bool,pub above:bool,pub speed:f64,pub object:Object}
 impl Note{fn plain(&self)->bool{!self.fake&&!matches!(self.kind,NoteKind::Hold{..})}}
 #[derive(PartialEq,PartialOrd)]struct Total(f64);impl Eq for Total{}impl Ord for Total{fn cmp(&self,other:&Self)->std::cmp::Ordering{self.0.total_cmp(&other.0)}}
 trait NotNanExt{fn not_nan(self)->Total;}impl NotNanExt for f64{fn not_nan(self)->Total{assert!(!self.is_nan());Total(self)}}
 include!(concat!(env!("OUT_DIR"),"/line_cache.rs"));
 pub struct Line{pub notes:Vec<Note>,pub cache:JudgeLineCache}
 pub struct Chart{pub lines:Vec<Line>}
 pub mod implementation{use super::Chart;include!(concat!(env!("OUT_DIR"),"/conversion.rs"));}
 pub fn check_cache(line:&Line){
  assert_eq!(line.cache.update_order.len(),line.notes.len());
  assert!(line.notes[..line.cache.not_plain_count].iter().all(|n|!n.plain()));
  assert!(line.notes[line.cache.not_plain_count..].iter().all(Note::plain));
  for &i in &line.cache.above_indices{assert!(line.notes[i].plain()&&line.notes[i].above);}
  for &i in &line.cache.below_indices{assert!(line.notes[i].plain()&&!line.notes[i].above);}
 }
}
#[test]fn repeated_conversions_rebuild_real_render_cache_and_keep_original_identity_after_sorting(){
 use core::*;use chart_play::NoteConversion as Mode;use judge::HitSound;
 let kinds=[NoteKind::Click,NoteKind::Hold{end_time:5.,end_height:8.},NoteKind::Drag,NoteKind::Flick];
 let mut notes:Vec<_>=kinds.iter().enumerate().map(|(i,k)|Note{kind:k.clone(),hitsound:HitSound::Custom,time:2.+i as f64,height:4.+i as f64,fake:i==3,above:i%2==0,speed:1.,object:Object{translation:(Anim,Anim)}}).collect();
 let cache=JudgeLineCache::new(&mut notes);
 let mut chart=Chart{lines:vec![Line{notes,cache}]};
 let mut backup=core::implementation::NoteConversionBackup::capture(&chart);
 for mode in [Mode::Tap,Mode::Original,Mode::Drag,Mode::Original,Mode::Flick,Mode::Tap,Mode::Original] {
  backup.apply(&mut chart,mode);check_cache(&chart.lines[0]);
  for n in &chart.lines[0].notes{
   let i=(n.time-2.)as usize;
   let target=match mode{Mode::Original=>kinds[i].clone(),Mode::Tap=>NoteKind::Click,Mode::Drag=>NoteKind::Drag,Mode::Flick=>NoteKind::Flick};
   let sound=if mode==Mode::Original{HitSound::Custom}else{HitSound::default_from_kind(&target)};
   assert_eq!(n.kind,target);assert_eq!(n.hitsound,sound);assert_eq!(n.height,4.+i as f64);assert_eq!(n.fake,i==3);assert_eq!(n.above,i%2==0);
  }
 }
}
