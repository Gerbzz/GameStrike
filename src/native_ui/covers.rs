use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
use crate::application::{GameCard, GameId};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct CoverTexture {
    pub handle: TextureHandle,
    pub synthetic: bool,
    revision: u64,
    bytes: usize,
    last_used: u64,
}

#[derive(Default)]
pub(super) struct Covers {
    pub textures: BTreeMap<GameId, CoverTexture>,
    clock: u64,
}

impl Covers {
    pub fn retain(&mut self, games: &[GameCard]) {
        let ids: BTreeSet<_> = games.iter().map(|game| &game.id).collect();
        self.textures.retain(|id,_| ids.contains(id));
    }

    pub fn prepare(&mut self, ctx: &Context, game: &GameCard, revision: u64) {
        self.clock=self.clock.saturating_add(1);
        if let Some(entry)=self.textures.get_mut(&game.id) {
            entry.last_used=self.clock;
            if entry.revision==revision { return; }
        }
        let real = game.cover.as_ref().and_then(|cover| {
            let [w,h]=cover.size;
            let length=w.checked_mul(h)?.checked_mul(4)?;
            if w==0 || h==0 || w>4096 || h>4096 || length!=cover.rgba.len() { return None; }
            let image = image::RgbaImage::from_raw(w as u32,h as u32,cover.rgba.clone())?;
            let image = if w>512 || h>688 {
                let scale=(512.0/w as f32).min(688.0/h as f32).min(1.0);
                image::imageops::resize(&image,(w as f32*scale).max(1.0) as u32,
                    (h as f32*scale).max(1.0) as u32,image::imageops::FilterType::Triangle)
            } else { image };
            Some(ColorImage::from_rgba_unmultiplied([image.width() as usize,image.height() as usize],image.as_raw()))
        });
        let synthetic=real.is_none();
        let color_image=real.unwrap_or_else(|| original_pattern(&game.id));
        let bytes=color_image.size[0]*color_image.size[1]*4;
        if let Some(entry)=self.textures.get_mut(&game.id) {
            entry.handle.set(color_image,TextureOptions::LINEAR);
            entry.synthetic=synthetic; entry.revision=revision;entry.bytes=bytes;entry.last_used=self.clock;
        } else {
            self.textures.insert(game.id.clone(),CoverTexture {
                handle:ctx.load_texture(format!("PWDS cover {:?}",game.id),color_image,TextureOptions::LINEAR),
                synthetic,revision,bytes,last_used:self.clock,
            });
        }
    }

    pub fn trim(&mut self) {
        let ids: Vec<_>=self.textures.keys().cloned().collect();
        let entries: Vec<_>=ids.iter().map(|id| {
            let entry=&self.textures[id];
            super::cache_budget::Resident {bytes:entry.bytes,last_used:entry.last_used}
        }).collect();
        // Covers are prepared only for the visible window. Recently seen textures
        // survive scrolling, bounded by both entry count and actual RGBA bytes.
        for index in super::cache_budget::eviction_indices(&entries,128,96*1024*1024) {
            self.textures.remove(&ids[index]);
        }
    }
}

/// Original mathematical fallback art. No catalog art or donor app pixels.
fn original_pattern(id: &GameId) -> ColorImage {
    let seed=format!("{id:?}").bytes().fold(2_166_136_261_u32,|v,b|(v^b as u32).wrapping_mul(16_777_619));
    let mut pixels=vec![0_u8;128*172*4];
    let tint=[70.0+((seed>>16)&127) as f32,55.0+((seed>>8)&127) as f32,65.0+(seed&127) as f32];
    for y in 0..172 {
        for x in 0..128 {
            let px=x as f32/128.0;let py=y as f32/172.0;
            let horizon=0.57+0.06*(px*13.0+(seed%19) as f32).sin()+0.05*(px*27.0).sin();
            let distance=((px-0.66).powi(2)+(py-0.29).powi(2)).sqrt();
            let orb=(1.0-(distance/0.12).clamp(0.0,1.0)).powf(0.25);
            let sky=(1.0-py)*0.5;
            let ridge=if py>horizon { 0.17+(py-horizon)*0.3 } else { 0.55+sky };
            let stripe=(1.0-((px*7.0+py*3.0).fract()-0.5).abs()*2.0).powf(18.0)*0.08;
            let noise=(seed^(x as u32).wrapping_mul(73856093)^(y as u32).wrapping_mul(19349663)).wrapping_mul(1664525);
            let star=if noise%997<3 && py<0.55 { 110.0 } else { 0.0 };
            let index=(y*128+x)*4;
            for channel in 0..3 {
                pixels[index+channel]=(tint[channel]*ridge+orb*(145.0-channel as f32*15.0)+stripe*220.0+star).clamp(0.0,255.0) as u8;
            }
            pixels[index+3]=255;
        }
    }
    ColorImage::from_rgba_unmultiplied([128,172],&pixels)
}

pub(super) fn logo(ctx: &Context) -> Result<TextureHandle,String> {
    // Existing project-owned PWDS branding only; native Rust PNG decoding.
    let mut image=image::load_from_memory(include_bytes!("../../assets/branding/PWDS-logo.png"))
        .map_err(|error|format!("PWDS bundled branding could not decode: {error}"))?.into_rgba8();
    for pixel in image.pixels_mut() {
        pixel.0[0]=255;pixel.0[1]=106;pixel.0[2]=26;
    }
    Ok(ctx.load_texture("PWDS original emblem",ColorImage::from_rgba_unmultiplied(
        [image.width() as usize,image.height() as usize],image.as_raw()),TextureOptions::LINEAR))
}
