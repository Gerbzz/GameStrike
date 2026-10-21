//! Native PWDS Collection view. This module returns application intents only.
//! It has no process, file-picker, browser, network, or audio-device authority.
pub mod boot;
pub mod geometry;
mod activation;
mod covers;
mod cache_budget;
mod gpu;
mod mesh_geometry;
mod navigation;
mod paint;

use std::sync::atomic::Ordering;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Id, Key, Painter,
    PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, TextureHandle, WidgetInfo, WidgetType};
use crate::application::{AppCommand, AppSnapshot, GameCard, GameId};
use boot::BootPose;
use geometry::{chamfer, Geometry};
use activation::activated;
use paint::{AMBER, LIGHT, MUTED, TEXT};

const SECTIONS: [&str;4]=["GAMES","SOUNDBOARD","VOICE","MUSIC"];
const FILTERS: [&str;3]=["ALL GAMES","FAVORITES","RECENTLY PLAYED"];

pub struct NativeView {
    gpu: gpu::NativeGpu,
    covers: covers::Covers,
    logo: TextureHandle,
    section: usize,
    filter: usize,
    selected: Option<GameId>,
    scroll: f32,
    epoch: u64,
    frame_signature: String,
    textures_prepared: bool,
}

impl NativeView {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self,String> {
        let state=cc.wgpu_render_state.as_ref().ok_or_else(||
            "PWDS requires the native wgpu renderer; no compatible render state was supplied".to_owned())?;
        let gpu=gpu::NativeGpu::new(state)?;
        let logo=covers::logo(&cc.egui_ctx)?;
        let mut visuals=egui::Visuals::dark();
        visuals.selection.bg_fill=AMBER;
        visuals.selection.stroke=Stroke::new(1.0,LIGHT);
        visuals.override_text_color=Some(TEXT);
        cc.egui_ctx.set_visuals(visuals);
        Ok(Self { gpu,covers:covers::Covers::default(),logo,section:0,filter:0,
            selected:None,scroll:0.0,epoch:0,frame_signature:String::new(),textures_prepared:false })
    }

    pub fn renderer_label(&self) -> String { self.gpu.label.clone() }

    /// True only after this exact content pose has prepared texture data and a
    /// native callback has recorded its foreground draw in a real render pass.
    /// The host must separately wait for capture/readback or presentation.
    pub fn content_ready(&self) -> bool {
        self.epoch>0 && self.textures_prepared
            && self.gpu.receipt.prepared.load(Ordering::Acquire)==self.epoch
            && self.gpu.receipt.painted.load(Ordering::Acquire)==self.epoch
    }

    pub fn show(&mut self, ctx: &egui::Context, snapshot: &AppSnapshot, pose: BootPose) -> Vec<AppCommand> {
        // Public pose fields cannot inject NaN into geometry or uniforms.
        let pose=BootPose::at(pose.seconds,false,false);
        let g=Geometry::new(ctx.content_rect(),ctx.pixels_per_point());
        let mut commands=Vec::new();
        self.covers.retain(&snapshot.games);
        self.textures_prepared=false;
        egui::Area::new(Id::new("pwds.native.collection"))
            .order(egui::Order::Background).fixed_pos(g.viewport.min).movable(false)
            .sense(Sense::hover()).fade_in(false).constrain(false)
            .default_size(g.viewport.size()).show(ctx,|ui| {
                ui.set_min_size(g.viewport.size());
                ui.set_clip_rect(g.viewport);
                let p=ui.painter().clone();
                p.add(self.gpu.callback(g.viewport,pose,false,0));
                let chrome=paint::faded(&p,pose.frame);
                paint::frame(&chrome,&g,pose);
                if pose.content>0.0 {
                    let content=paint::faded(&p,pose.content);
                    self.sidebar(ui,&content,&g,snapshot,pose.settled());
                    self.heading(&content,&g,snapshot);
                }
                let visible=self.visible(snapshot);
                if !visible.iter().any(|&i|Some(&snapshot.games[i].id)==self.selected.as_ref()) {
                    self.selected=visible.first().map(|&i|snapshot.games[i].id.clone());
                }
                self.scroll=self.scroll.clamp(0.0,g.max_scroll(visible.len()));
                if self.section==0 && pose.settled() {
                    self.keyboard(ctx,&g,snapshot,&visible,&mut commands);
                    if ctx.input(|i|i.pointer.hover_pos().is_some_and(|point|g.library.contains(point))) {
                        self.scroll=(self.scroll-ctx.input(|i|i.smooth_scroll_delta.y)).clamp(0.0,g.max_scroll(visible.len()));
                    }
                }
                if self.section==0 {
                    // Prepare actual CPU image data even during the black ignition
                    // frame; channel enqueue alone never sets content_ready.
                    for (position,&i) in visible.iter().enumerate() {
                        let rect=g.card_rect(position,self.scroll);
                        if rect.intersects(g.library) { self.covers.prepare(ctx,&snapshot.games[i],snapshot.revision); }
                    }
                    self.covers.trim();
                    self.textures_prepared=visible.iter().enumerate().all(|(position,&i)| {
                        !g.card_rect(position,self.scroll).intersects(g.library)
                            || self.covers.textures.contains_key(&snapshot.games[i].id)
                    });
                    if pose.content>0.0 {
                        let content=paint::faded(&p,pose.content);
                        self.games(ui,&content,&g,snapshot,&visible,pose.settled(),&mut commands);
                    }
                } else {
                    self.textures_prepared=true;
                    if pose.content>0.0 { self.unavailable(&paint::faded(&p,pose.content),&g); }
                }
                if pose.content>0.0 {
                    self.bottom(ui,&paint::faded(&p,pose.content),&g,snapshot,pose.settled(),&mut commands);
                }
                paint::emblem(&p,&g,pose,self.logo.id());
                if !pose.settled() && pose.seconds>=3.3 && pose.seconds<4.65 {
                    let alpha=boot::smooth(3.3,3.65,pose.seconds)*(1.0-boot::smooth(4.2,4.65,pose.seconds));
                    let at=pos2(g.viewport.center().x,g.viewport.top()+g.viewport.height()*0.65);
                    paint::faded(&p,alpha).text(at,Align2::CENTER_CENTER,"GAMESTRIKE",
                        FontId::monospace(42.0*g.scale.max(0.5)),Color32::from_rgb(255,196,122));
                }
                let signature=format!("{}|{}|{:?}|{}|{}|{:?}|{}|{}|{:?}",snapshot.revision,snapshot.games.len(),
                    g.viewport,self.section,self.filter,self.selected,self.scroll,g.pixels_per_point,pose);
                if self.frame_signature!=signature {
                    self.epoch=self.epoch.checked_add(1).unwrap_or(1);
                    self.frame_signature=signature;
                }
                // Last callback: after covers, native text, focus rings and emblem.
                p.add(self.gpu.callback(g.viewport,pose,true,self.epoch));
            });
        // The host owns live animation/capture cadence. Frozen poses become idle
        // once the actual render callback has acknowledged their content.
        if !self.content_ready() { ctx.request_repaint(); }
        commands
    }

    fn visible(&self,snapshot: &AppSnapshot) -> Vec<usize> {
        let mut indices: Vec<_>=snapshot.games.iter().enumerate().filter_map(|(i,game)| {
            (match self.filter { 1=>game.favorite,2=>game.last_played.is_some(),_=>true }).then_some(i)
        }).collect();
        if self.filter==2 { indices.sort_by_key(|&i|std::cmp::Reverse(snapshot.games[i].last_played)); }
        indices
    }

    fn heading(&self,p: &Painter,g: &Geometry,snapshot: &AppSnapshot) {
        let title=if self.section==0 { "GAME COLLECTION" } else { SECTIONS[self.section] };
        let clip=p.with_clip_rect(g.header);
        clip.text(g.header.left_center()+vec2(18.0*g.scale,0.0),Align2::LEFT_CENTER,title,
            FontId::monospace(37.0*g.scale.max(0.48)),Color32::from_rgb(245,140,57));
        let info=if snapshot.refreshing { "REFRESHING COLLECTION".to_owned() }
            else { format!("{} GAMES  /  {}",snapshot.games.len(),FILTERS[self.filter]) };
        let available=Rect::from_min_max(pos2(g.header.right()+60.0*g.scale,g.header.top()),
            pos2(g.frame.right()-65.0*g.scale,g.header.bottom()));
        if available.is_positive() {
            p.with_clip_rect(available).text(available.center(),Align2::CENTER_CENTER,info,
                FontId::monospace((13.0*g.scale).max(9.0)),MUTED);
        }
    }

    fn sidebar(&mut self,ui: &mut egui::Ui,p: &Painter,g: &Geometry,snapshot: &AppSnapshot,enabled: bool) {
        let focused_at_start=ui.memory(|memory|memory.focused());
        for (i,label) in SECTIONS.iter().enumerate() {
            let rect=g.navigation[i];
            let response=control(ui,p,rect,nav_id(i),label,enabled,self.section==i,8.0*g.scale);
            glyph(p,pos2(rect.left()+19.0*g.scale,rect.center().y),11.0*g.scale,i,
                if self.section==i { Color32::from_rgb(22,6,0) } else { TEXT });
            if activated(ui,&response,rect,8.0*g.scale,enabled) { self.section=i;self.scroll=0.0;response.request_focus(); }
            if enabled && focused_at_start==Some(response.id) {
                let action=ui.input_mut(|input|navigation::take_nav_action(input,i));
                if let Some(action)=action {
                    // egui also schedules an automatic directional move at end_pass.
                    // Clear it after taking ownership of this key, then request one target.
                    ui.memory_mut(|memory|memory.move_focus(egui::FocusDirection::None));
                    match action {
                        navigation::NavAction::Select(next)=>{
                            self.section=next;self.scroll=0.0;
                            ui.memory_mut(|memory|memory.request_focus(nav_id(next)));
                        },
                        navigation::NavAction::EnterGames=>if let Some(id)=&self.selected {
                            ui.memory_mut(|memory|memory.request_focus(card_id(id)));
                        },
                    }
                }
            }
        }
        for (i,label) in FILTERS.iter().enumerate() {
            let rect=g.filters[i];
            let count=match i { 1=>snapshot.games.iter().filter(|game|game.favorite).count(),
                2=>snapshot.games.iter().filter(|game|game.last_played.is_some()).count(),_=>snapshot.games.len() };
            let response=control(ui,p,rect,Id::new(("pwds.filter",i)),label,enabled,
                self.section==0 && self.filter==i,7.0*g.scale);
            p.with_clip_rect(rect).text(rect.right_center()-vec2(8.0*g.scale,0.0),Align2::RIGHT_CENTER,
                count.to_string(),FontId::monospace((13.0*g.scale).max(9.0)),
                if self.section==0 && self.filter==i { Color32::from_rgb(44,15,0) } else { MUTED });
            if activated(ui,&response,rect,7.0*g.scale,enabled) {
                self.section=0;self.filter=i;self.scroll=0.0;response.request_focus();
            }
        }
    }

    fn keyboard(&mut self,ctx: &egui::Context,g: &Geometry,snapshot: &AppSnapshot,
        visible: &[usize],commands: &mut Vec<AppCommand>) {
        let focused=ctx.memory(|m|m.focused());
        let grid_focused=focused.is_none() || visible.iter().any(|&i|Some(card_id(&snapshot.games[i].id))==focused);
        if ctx.input(|i|i.key_pressed(Key::R) && !i.modifiers.command && !i.modifiers.alt) && !snapshot.refreshing {
            commands.push(AppCommand::Refresh);
        }
        if !grid_focused || visible.is_empty() { return; }
        let index=visible.iter().position(|&i|Some(&snapshot.games[i].id)==self.selected.as_ref()).unwrap_or(0);
        let action=ctx.input_mut(|input|navigation::take_grid_action(input,index,visible.len(),g.columns));
        let next=match action {
            Some(navigation::GridAction::Select(next))=>Some(next),
            Some(navigation::GridAction::Sidebar)=>{
                ctx.memory_mut(|memory|{
                    memory.move_focus(egui::FocusDirection::None);memory.request_focus(nav_id(0));
                });
                return;
            },
            None=>None,
        };
        if let Some(next)=next {
            let id=snapshot.games[visible[next]].id.clone();
            self.selected=Some(id.clone());self.scroll=g.reveal_scroll(next,visible.len(),self.scroll);
            ctx.memory_mut(|memory|{
                memory.move_focus(egui::FocusDirection::None);memory.request_focus(card_id(&id));
            });
        }
        let game=&snapshot.games[visible[next.unwrap_or(index)]];
        if ctx.input(|i|i.key_pressed(Key::Enter)) && game.files_ok && !snapshot.running.contains(&game.id) {
            commands.push(AppCommand::Launch(game.id.clone()));
        }
        if ctx.input(|i|i.key_pressed(Key::F) && !i.modifiers.command && !i.modifiers.alt) {
            commands.push(AppCommand::ToggleFavorite(game.id.clone(),!game.favorite));
        }
    }

    fn games(&mut self,ui: &mut egui::Ui,p: &Painter,g: &Geometry,snapshot: &AppSnapshot,
        visible: &[usize],enabled: bool,commands: &mut Vec<AppCommand>) {
        let well=p.with_clip_rect(g.library);
        if visible.is_empty() {
            well.text(g.library.center()-vec2(0.0,16.0),Align2::CENTER_CENTER,"NO GAMES IN THIS COLLECTION",
                FontId::monospace((24.0*g.scale).max(12.0)),LIGHT);
            well.text(g.library.center()+vec2(0.0,13.0),Align2::CENTER_CENTER,
                if snapshot.refreshing { "Reading the local library…" } else { "Choose another collection or refresh the library." },
                FontId::proportional((18.0*g.scale).max(11.0)),MUTED);
            return;
        }
        for (position,&i) in visible.iter().enumerate() {
            let game=&snapshot.games[i];let rect=g.card_rect(position,self.scroll);
            if !rect.intersects(g.library) { continue; }
            let response=ui.interact(rect.intersect(g.library),card_id(&game.id),if enabled { Sense::click() } else { Sense::hover() });
            let selected=self.selected.as_ref()==Some(&game.id);
            response.widget_info(||WidgetInfo::selected(WidgetType::SelectableLabel,enabled,selected,&game.title));
            let pointer_hit=ui.input(|input|input.pointer.interact_pos())
                .and_then(|point|g.card_at(point,visible.len(),self.scroll))==Some(position);
            if enabled && pointer_hit && response.clicked_by(PointerButton::Primary) {
                self.selected=Some(game.id.clone());response.request_focus();ui.ctx().request_repaint();
            }
            if enabled && activation::clicked_without_pointer(ui,&response) {
                self.selected=Some(game.id.clone());response.request_focus();
            }
            if enabled && response.has_focus() && self.selected.as_ref()!=Some(&game.id) {
                self.selected=Some(game.id.clone());ui.ctx().request_repaint();
            }
            if enabled && pointer_hit && response.double_clicked() && game.files_ok && !snapshot.running.contains(&game.id) {
                commands.push(AppCommand::Launch(game.id.clone()));
            }
            self.card(&well,g,rect,game,selected,response.has_focus(),snapshot.running.contains(&game.id));
            response.on_hover_text(format!("{}\n{}\n{}",game.title,game.source,
                if !game.files_ok { "Game files unavailable" } else if snapshot.running.contains(&game.id) { activity(game) }
                else { "Enter or double-click to launch · F to favorite" }));
        }
        let max=g.max_scroll(visible.len());
        if max>0.0 {
            let track=Rect::from_min_max(pos2(g.library.right()-5.0*g.scale,g.library.top()+10.0*g.scale),
                pos2(g.library.right()-2.0*g.scale,g.library.bottom()-10.0*g.scale));
            let content_height=g.library.height()+max;
            let height=(track.height()*g.library.height()/content_height).max(16.0*g.scale);
            let thumb=Rect::from_min_size(pos2(track.left(),track.top()+(track.height()-height)*self.scroll/max),vec2(track.width(),height));
            well.rect_filled(track,0.0,Color32::from_rgb(47,20,7));well.rect_filled(thumb,0.0,AMBER.gamma_multiply(0.7));
            let response=ui.interact(thumb.expand(3.0*g.scale),Id::new("pwds.scroll"),if enabled { Sense::drag() } else { Sense::hover() });
            if response.dragged() { self.scroll=(self.scroll+response.drag_delta().y*max/(track.height()-height).max(1.0)).clamp(0.0,max); }
        }
    }

    fn card(&self,p: &Painter,g: &Geometry,rect: Rect,game: &GameCard,selected: bool,focused: bool,running: bool) {
        let cut=g.card_cut();
        paint::plate(p,&chamfer(rect,[cut;4]),(3.6*g.scale).max(1.2),
            if selected { Color32::from_rgb(153,61,7) } else { Color32::from_rgb(32,14,6) },selected);
        let art=rect.shrink(6.0*g.scale);
        if let Some(texture)=self.covers.textures.get(&game.id) {
            let size=texture.handle.size();let ratio=size[0] as f32/size[1] as f32;
            let target=art.width()/art.height();
            let uv=if ratio>target {
                let width=target/ratio;Rect::from_min_max(pos2((1.0-width)*0.5,0.0),pos2((1.0+width)*0.5,1.0))
            } else {
                let height=ratio/target;Rect::from_min_max(pos2(0.0,(1.0-height)*0.5),pos2(1.0,(1.0+height)*0.5))
            };
            paint::image(p,art,cut*0.6,texture.handle.id(),uv,Color32::WHITE);
            let title=Rect::from_min_max(pos2(art.left(),art.top()+art.height()*0.68),art.max);
            paint::polygon(p,&chamfer(title,[0.0,0.0,cut*0.6,cut*0.6]),
                Color32::from_black_alpha(115),Color32::from_black_alpha(235));
            let title_p=p.with_clip_rect(title.shrink(4.0*g.scale));
            let font=FontId::proportional((18.0*g.scale).max(10.0));
            let galley=title_p.layout(game.title.to_uppercase(),font,TEXT,title.width()-10.0*g.scale);
            title_p.galley(pos2(title.left()+5.0*g.scale,title.top()+5.0*g.scale),galley,TEXT);
            let label=if texture.synthetic { "GENERATED COVER" } else { game.source.as_str() };
            p.with_clip_rect(art).text(art.left_bottom()+vec2(5.0,-6.0)*g.scale,Align2::LEFT_BOTTOM,label,
                FontId::monospace((9.0*g.scale).max(7.0)),MUTED);
        }
        if game.favorite {
            let badge=Rect::from_min_size(art.right_top()-vec2(26.0*g.scale,0.0),vec2(26.0,24.0)*g.scale);
            paint::polygon(p,&chamfer(badge,[0.0,0.0,5.0*g.scale,0.0]),AMBER,Color32::from_rgb(242,106,12));
            star(p,badge.center(),7.0*g.scale,Color32::from_rgb(25,6,0));
        }
        if !game.files_ok || running {
            let status=if running { activity(game) } else { "FILES UNAVAILABLE" };
            let badge=Rect::from_min_max(art.min,pos2(art.right(),art.top()+23.0*g.scale));
            p.rect_filled(badge,0.0,Color32::from_black_alpha(220));
            p.with_clip_rect(badge).text(badge.center(),Align2::CENTER_CENTER,status,
                FontId::monospace((11.0*g.scale).max(8.0)),if running { LIGHT } else { Color32::from_rgb(255,180,168) });
        }
        if selected || focused {
            p.add(Shape::closed_line(chamfer(rect.shrink(1.0),[cut;4]),
                Stroke::new(if focused { 2.6 } else { 1.8 },if focused { LIGHT } else { AMBER })));
        }
    }

    fn unavailable(&self,p: &Painter,g: &Geometry) {
        let p=p.with_clip_rect(g.library.shrink(18.0*g.scale));
        let title=match self.section { 1=>"SOUNDBOARD UNAVAILABLE",2=>"VOICE ROUTING UNAVAILABLE",_=>"MUSIC PLAYBACK UNAVAILABLE" };
        let at=g.library.center();
        p.text(at-vec2(0.0,28.0*g.scale),Align2::CENTER_CENTER,title,
            FontId::monospace((28.0*g.scale).max(13.0)),LIGHT);
        p.text(at+vec2(0.0,7.0*g.scale),Align2::CENTER_CENTER,"Audio is unavailable in this preview.",
            FontId::proportional((21.0*g.scale).max(12.0)),MUTED);
        p.text(at+vec2(0.0,39.0*g.scale),Align2::CENTER_CENTER,"Choose Games to return to your collection.",
            FontId::proportional((17.0*g.scale).max(11.0)),MUTED);
    }

    fn bottom(&self,ui: &mut egui::Ui,p: &Painter,g: &Geometry,snapshot: &AppSnapshot,
        enabled: bool,commands: &mut Vec<AppCommand>) {
        let selected=self.selected.as_ref().and_then(|id|snapshot.games.iter().find(|game|&game.id==id));
        let title_rect=Rect::from_min_max(g.bottom.min+vec2(3.0,8.0)*g.scale,
            pos2(g.launch.left()-30.0*g.scale,g.bottom.bottom()-7.0*g.scale));
        let text=p.with_clip_rect(title_rect);
        if self.section==0 {
            if let Some(game)=selected {
                text.text(title_rect.left_top(),Align2::LEFT_TOP,game.title.to_uppercase(),
                    FontId::monospace((24.0*g.scale).max(11.0)),Color32::from_rgb(255,196,122));
                let status=if snapshot.running.contains(&game.id) { activity(game) }
                    else if game.files_ok { "READY TO PLAY" } else { "FILES UNAVAILABLE" };
                text.text(title_rect.left_bottom(),Align2::LEFT_BOTTOM,format!("{}  /  {}",status,game.source.to_uppercase()),
                    FontId::monospace((13.0*g.scale).max(9.0)),MUTED);
            } else {
                text.text(title_rect.center(),Align2::CENTER_CENTER,"SELECT A GAME",
                    FontId::monospace((22.0*g.scale).max(11.0)),MUTED);
            }
        } else {
            text.text(title_rect.center(),Align2::CENTER_CENTER,"AUDIO UNAVAILABLE",
                FontId::monospace((22.0*g.scale).max(11.0)),MUTED);
        }
        let can_launch=enabled && self.section==0 && selected.is_some_and(|game|game.files_ok && !snapshot.running.contains(&game.id));
        let launch=control(ui,p,g.launch,Id::new("pwds.launch"),"LAUNCH",can_launch,false,7.0*g.scale);
        if activated(ui,&launch,g.launch,7.0*g.scale,can_launch) {
            if let Some(game)=selected { commands.push(AppCommand::Launch(game.id.clone())); }
        }
        let can_favorite=enabled && self.section==0 && selected.is_some();
        let favorite=control(ui,p,g.favorite,Id::new("pwds.favorite"),
            if selected.is_some_and(|game|game.favorite) { "UNFAVORITE" } else { "FAVORITE" },can_favorite,false,7.0*g.scale);
        if activated(ui,&favorite,g.favorite,7.0*g.scale,can_favorite) {
            if let Some(game)=selected { commands.push(AppCommand::ToggleFavorite(game.id.clone(),!game.favorite)); }
        }
        let can_refresh=enabled && !snapshot.refreshing;
        let refresh=control(ui,p,g.refresh,Id::new("pwds.refresh"),
            if snapshot.refreshing { "READING…" } else { "REFRESH" },can_refresh,false,7.0*g.scale);
        if activated(ui,&refresh,g.refresh,7.0*g.scale,can_refresh) { commands.push(AppCommand::Refresh); }
        if !snapshot.problems.is_empty() {
            let rect=Rect::from_min_size(pos2(g.library.left()+8.0*g.scale,g.library.bottom()-24.0*g.scale),
                vec2(g.library.width()-16.0*g.scale,20.0*g.scale));
            p.rect_filled(rect,0.0,Color32::from_black_alpha(225));
            p.with_clip_rect(rect).text(rect.left_center()+vec2(5.0*g.scale,0.0),Align2::LEFT_CENTER,
                format!("{} LIBRARY NOTICE(S): {}",snapshot.problems.len(),snapshot.problems[0]),
                FontId::monospace((12.0*g.scale).max(9.0)),Color32::from_rgb(255,180,168));
            ui.interact(rect,Id::new("pwds.problems"),Sense::hover()).on_hover_text(snapshot.problems.join("\n"));
        }
    }
}

fn nav_id(index: usize) -> Id { Id::new(("pwds.section",index)) }
fn card_id(id: &GameId) -> Id { Id::new(("pwds.card",format!("{id:?}"))) }

fn control(ui: &egui::Ui,p: &Painter,rect: Rect,id: Id,label: &str,enabled: bool,selected: bool,cut: f32) -> Response {
    let response=ui.interact(rect,id,if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(||WidgetInfo::selected(WidgetType::Button,enabled,selected,label));
    let painter=paint::faded(p,if enabled { 1.0 } else { 0.43 });
    paint::slab(&painter,rect,cut,selected,response.has_focus());
    let label_rect=rect.shrink2(vec2(rect.width()*0.12,0.0));
    painter.with_clip_rect(label_rect).text(rect.center()+vec2(if id==nav_id(0)||id==nav_id(1)||id==nav_id(2)||id==nav_id(3) { rect.width()*0.05 } else { 0.0 },0.0),
        Align2::CENTER_CENTER,label,FontId::proportional((rect.height()*0.37).max(9.0)),
        if selected { Color32::from_rgb(22,6,0) } else { TEXT });
    response
}

fn activity(game: &GameCard) -> &'static str {
    match &game.id { GameId::Store(_)=>"OPENING",GameId::Html(_)=>"RUNNING" }
}

fn glyph(p: &Painter,at: Pos2,radius: f32,index: usize,color: Color32) {
    let stroke=Stroke::new(1.4,color);
    match index {
        0=>for x in [-0.55,0.55] { for y in [-0.55,0.55] {
            p.rect_stroke(Rect::from_center_size(at+vec2(x,y)*radius,vec2(radius*0.7,radius*0.7)),0.0,stroke,egui::StrokeKind::Inside);
        } },
        1=>for (i,height) in [0.4,0.9,0.65,1.0].iter().enumerate() {
            let x=at.x+(i as f32-1.5)*radius*0.45;
            p.line_segment([pos2(x,at.y-radius*height),pos2(x,at.y+radius*height)],stroke);
        },
        2=>{ p.circle_stroke(at-vec2(0.0,radius*0.3),radius*0.42,stroke);
            p.line_segment([at,at+vec2(0.0,radius)],stroke);
            p.line_segment([at+vec2(-0.5,1.0)*radius,at+vec2(0.5,1.0)*radius],stroke); },
        _=>{ p.line_segment([at+vec2(-0.3,0.8)*radius,at+vec2(-0.3,-0.8)*radius],stroke);
            p.line_segment([at+vec2(-0.3,-0.8)*radius,at+vec2(0.7,-0.45)*radius],stroke);
            p.circle_filled(at+vec2(-0.6,0.8)*radius,radius*0.35,color); },
    }
}

fn star(p: &Painter,at: Pos2,radius: f32,color: Color32) {
    let points: Vec<_>=(0..10).map(|i| {
        let a=i as f32*std::f32::consts::PI/5.0-std::f32::consts::FRAC_PI_2;
        at+vec2(a.cos(),a.sin())*radius*if i%2==0 { 1.0 } else { 0.45 }
    }).collect();
    paint::polygon(p,&points,color,color);
}
