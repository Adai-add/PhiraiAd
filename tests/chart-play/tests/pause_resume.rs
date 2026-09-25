//! Run the actual shared game pause/resume functions with an audio recorder and
//! the production TimeManager. No model of the count-in is duplicated here.
#![allow(dead_code)]
use anyhow::Result;
use phira_chart_play_tests::config;
use std::{cell::Cell,rc::Rc};
#[path="../../../prpr/src/time.rs"]mod time;
use time::TimeManager;
#[derive(PartialEq)]enum GameMode{Normal,Exercise}
enum State{Playing,BeforeMusic}
enum TouchDebugClearSource{PauseButton,AutoFlipTransition}
struct Judge{clears:usize}impl Judge{fn clear_touch_input_with_source(&mut self,_:TouchDebugClearSource){self.clears+=1;}}
struct Music{at:f64,paused:bool,pitch:bool}
impl Music{
 fn paused(&mut self)->bool{self.paused}fn pause(&mut self)->Result<()>{self.paused=true;Ok(())}
 fn play(&mut self)->Result<()>{self.paused=false;Ok(())}fn position(&self)->f64{self.at}
 fn seek_to(&mut self,t:f64)->Result<()>{self.at=t;Ok(())}fn preserves_pitch(&self)->bool{self.pitch}
}
struct Resource{config:config::Config,time:f64}
struct GameScene{res:Resource,music:Music,mode:GameMode,state:State,exercise_range:std::ops::Range<f64>,exercise_state_reset_pending:bool,pause_rewind:Option<f64>,reset:Option<f64>}
impl GameScene{
 fn new_music(res:&mut Resource,_:&GameMode)->Result<Music>{Ok(Music{at:0.,paused:true,pitch:res.config.practice_preserve_pitch})}
 fn reset_exercise_judgement_to(&mut self,t:f64){self.reset=Some(t);}
}
include!(concat!(env!("OUT_DIR"),"/pause_resume.rs"));
fn scene()->GameScene{GameScene{res:Resource{config:config::Config::default(),time:5.},music:Music{at:5.,paused:false,pitch:false},mode:GameMode::Normal,state:State::Playing,exercise_range:0. ..10.,exercise_state_reset_pending:false,pause_rewind:None,reset:None}}
#[test]fn automatic_pause_freezes_then_uses_exact_boundary_and_original_count_in(){
 let wall=Rc::new(Cell::new(0.));let w=wall.clone();let mut tm=TimeManager::manual(Box::new(move||w.get()));tm.seek_to(5.);
 let mut g=scene();let mut j=Judge{clears:0};GameScene::pause_playback(&mut g.music,&mut tm,&mut j,TouchDebugClearSource::AutoFlipTransition).unwrap();
 wall.set(0.8);assert_eq!(tm.now(),5.);assert!(g.music.paused);assert_eq!(j.clears,1);
 g.music.at=4.9;g.resume_from_pause(&mut tm,Some(5.)).unwrap();
 assert!(!tm.paused());assert!((tm.now()-2.).abs()<1e-10);assert_eq!(g.music.at,2.);assert!(!g.music.paused);assert!(g.pause_rewind.is_some());
 wall.set(3.8);assert!((tm.now()-5.).abs()<1e-10);
}
#[test]fn early_boundary_uses_before_music_and_manual_resume_remains_available(){
 let mut tm=TimeManager::manual(Box::new(||0.));tm.seek_to(1.);tm.pause();let mut g=scene();g.res.time=1.;g.music.at=1.;
 g.resume_from_pause(&mut tm,None).unwrap();assert_eq!(tm.now(),-2.);assert!(g.music.paused);assert!(matches!(g.state,State::BeforeMusic));
}
#[test]fn practice_speed_pitch_and_pending_reset_survive_shared_resume(){
 let mut tm=TimeManager::manual(Box::new(||0.));tm.seek_to(5.);tm.pause();let mut g=scene();g.mode=GameMode::Exercise;g.res.config.speed=0.5;g.res.config.practice_preserve_pitch=true;g.exercise_state_reset_pending=true;
 g.resume_from_pause(&mut tm,None).unwrap();assert_eq!(tm.speed,0.5);assert_eq!(tm.now(),2.);assert!(g.music.pitch);assert_eq!(g.reset,Some(2.));assert!(!g.exercise_state_reset_pending);
}
