use eframe::egui::{self, pos2, vec2, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureId};
use super::{boot::BootPose, geometry::{chamfer, Geometry}};
use super::mesh_geometry::{clean_polygon,triangles};

pub(super) const AMBER: Color32 = Color32::from_rgb(255, 138, 23);
pub(super) const LIGHT: Color32 = Color32::from_rgb(255, 226, 180);
pub(super) const TEXT: Color32 = Color32::from_rgb(255, 246, 238);
pub(super) const MUTED: Color32 = Color32::from_rgb(226, 196, 173);

pub(super) fn faded(p: &Painter, opacity: f32) -> Painter {
    let mut painter=p.clone();
    painter.multiply_opacity(opacity);
    painter
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let a = a.to_array(); let b = b.to_array();
    let c: [u8; 4] = std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32-a[i] as f32)*t.clamp(0.0,1.0)) as u8);
    Color32::from_rgba_premultiplied(c[0],c[1],c[2],c[3])
}
pub(super) fn polygon(p: &Painter, points: &[Pos2], top: Color32, bottom: Color32) {
    let cleaned=clean_polygon(points);
    let points=cleaned.as_slice();
    if points.len()<3 { return; }
    let min_y=points.iter().map(|v|v.y).fold(f32::INFINITY,f32::min);
    let max_y=points.iter().map(|v|v.y).fold(f32::NEG_INFINITY,f32::max);
    let mut mesh=Mesh::default();
    for v in points { mesh.colored_vertex(*v,mix(top,bottom,(v.y-min_y)/(max_y-min_y).max(0.001))); }
    for [a,b,c] in triangles(points) { mesh.add_triangle(a as u32,b as u32,c as u32); }
    p.add(Shape::mesh(mesh));
}

fn inset(points: &[Pos2], amount: f32) -> Vec<Pos2> {
    (0..points.len()).map(|i| {
        let prev=points[(i+points.len()-1)%points.len()]; let v=points[i]; let next=points[(i+1)%points.len()];
        let a=(v-prev).normalized(); let b=(next-v).normalized();
        let na=vec2(-a.y,a.x); let nb=vec2(-b.y,b.x);
        let sum=na+nb;
        v + sum*(amount/(1.0+na.dot(nb)).max(0.15))
    }).collect()
}

pub(super) fn plate(p: &Painter, points: &[Pos2], bevel: f32, face: Color32, lit: bool) {
    let cleaned=clean_polygon(points);
    let points=cleaned.as_slice();
    if points.len()<3 { return; }
    let shadow: Vec<_>=points.iter().map(|v|*v+vec2(bevel*0.5,bevel*1.4)).collect();
    polygon(p,&shadow,Color32::from_black_alpha(190),Color32::from_black_alpha(220));
    let inner=inset(points,bevel);
    let mut mesh=Mesh::default();
    for i in 0..points.len() {
        let j=(i+1)%points.len();
        let edge=(points[j]-points[i]).normalized();
        let illumination=(0.48+edge.x*0.35-edge.y*0.23).clamp(0.0,1.0);
        let rim=mix(Color32::from_rgb(76,31,8),LIGHT,illumination);
        let base=mesh.vertices.len() as u32;
        mesh.colored_vertex(points[i],rim); mesh.colored_vertex(points[j],rim);
        mesh.colored_vertex(inner[j],mix(face,AMBER,0.24)); mesh.colored_vertex(inner[i],mix(face,AMBER,0.24));
        mesh.add_triangle(base,base+1,base+2); mesh.add_triangle(base,base+2,base+3);
    }
    p.add(Shape::mesh(mesh));
    polygon(p,&inner,mix(face,Color32::from_rgb(93,46,19),0.30),face);
    if lit {
        for (width,alpha) in [(bevel*2.8,12),(bevel*1.5,22),(1.1,165)] {
            p.add(Shape::closed_line(points.to_vec(),Stroke::new(width,Color32::from_rgba_unmultiplied(255,117,24,alpha))));
        }
    }
    p.add(Shape::closed_line(inner,Stroke::new(0.8,Color32::from_rgb(113,54,22))));
}

pub(super) fn slab(p: &Painter, rect: Rect, cut: f32, selected: bool, focus: bool) {
    let points=chamfer(rect,[cut*0.5,cut,0.0,cut*0.5]);
    if selected {
        plate(p,&points,2.2,Color32::from_rgb(242,106,12),true);
        polygon(p,&chamfer(rect.shrink(2.2),[cut*0.4,cut,0.0,cut*0.4]),
            Color32::from_rgb(255,195,94),Color32::from_rgb(254,116,17));
    } else { plate(p,&points,1.6,Color32::from_rgb(18,8,4),focus); }
    if focus { p.add(Shape::closed_line(chamfer(rect.shrink(1.0),[cut;4]),Stroke::new(2.0,LIGHT))); }
}

pub(super) fn frame(p: &Painter, g: &Geometry, pose: BootPose) {
    if pose.frame<=0.0 { return; }
    let u=g.scale; let f=g.frame; let w=f.width(); let h=f.height(); let c=52.0*u;
    let top=g.header.top()+g.header.height()*0.3;
    let x=f.left(); let y=f.top();
    let outer=vec![pos2(x,top+c*1.6),pos2(x+c*0.62,top+c*0.9),pos2(x+c*0.62,top),
        pos2(f.right()-c,top),pos2(f.right(),top+c),pos2(f.right(),y+h-c*1.4),
        pos2(f.right()-c*0.6,y+h-c*0.8),pos2(f.right()-c*0.6,y+h),
        pos2(x+c,y+h),pos2(x,y+h-c)];
    // Assembly is a deterministic depth projection around the frame centre.
    let zoom=0.82+0.18*pose.frame;
    let p=p.with_clip_rect(g.viewport);
    let assembled: Vec<_>=outer.iter().map(|v|f.center()+(*v-f.center())*zoom).collect();
    plate(&p,&assembled,7.0*u,Color32::from_rgb(25,11,5),true);
    if pose.frame<0.45 { return; }
    let interior=Rect::from_min_max(pos2(g.sidebar.left()-18.0*u,g.library.top()-12.0*u),
        g.library.max+vec2(14.0,12.0)*u);
    plate(&p,&chamfer(interior,[18.0*u,30.0*u,26.0*u,18.0*u]),5.0*u,Color32::from_rgb(9,3,1),true);
    let glass=Rect::from_min_max(pos2(g.header.right()+40.0*u,top+7.0*u),pos2(f.right()-c-26.0*u,interior.top()-8.0*u));
    if glass.is_positive() { plate(&p,&chamfer(glass,[0.0,10.0*u,0.0,10.0*u]),2.0*u,Color32::from_rgb(48,20,7),false); }
    let side=g.sidebar.expand2(vec2(9.0,12.0)*u);
    plate(&p,&chamfer(side,[16.0*u,24.0*u,16.0*u,16.0*u]),5.0*u,Color32::from_rgb(14,6,3),true);
    for fraction in [0.16,0.60] {
        let ey=side.top()+side.height()*fraction;
        let ear=vec![pos2(side.left(),ey),pos2(side.left()-13.0*u,ey+9.0*u),
            pos2(side.left()-13.0*u,ey+64.0*u),pos2(side.left(),ey+73.0*u)];
        plate(&p,&ear,2.0*u,Color32::from_rgb(35,15,5),false);
        for dy in [22.0,51.0] { rivet(&p,pos2(side.left()-5.0*u,ey+dy*u),3.0*u); }
    }
    plate(&p,&chamfer(g.library.expand(6.0*u),[12.0*u,20.0*u,20.0*u,12.0*u]),
        4.0*u,Color32::from_rgb(7,2,1),true);
    let x0=g.emblem.left()+g.emblem.width()*0.6; let x1=g.header.right()+22.0*u;
    let y0=g.header.top()-6.0*u; let y1=g.header.bottom()+4.0*u; let tip=(y1-y0)*0.5;
    let tab=vec![pos2(x0,y0+10.0*u),pos2(x0+12.0*u,y0),pos2(x1-tip*0.3,y0),
        pos2(x1+tip*0.7,y0+tip),pos2(x1-tip*0.3,y1),pos2(x0+12.0*u,y1)];
    plate(&p,&tab,5.0*u,Color32::from_rgb(28,13,7),true);
    p.add(Shape::line(vec![pos2(x1+tip*0.95,y0+tip),pos2(x1+tip*1.5,top+4.0*u),
        pos2(f.right()-c*1.2,top+4.0*u)],Stroke::new(1.4*u,AMBER.gamma_multiply(0.55))));
    let b=g.bottom; let mid=b.center().y; let bx=b.left()-40.0*u; let right=g.launch.left()-22.0*u;
    let title=vec![pos2(bx,mid),pos2(bx+28.0*u,b.top()),pos2(right-26.0*u,b.top()),
        pos2(right,b.top()+26.0*u),pos2(right,b.bottom()-18.0*u),pos2(right-18.0*u,b.bottom()),pos2(bx+28.0*u,b.bottom())];
    if right>bx+50.0*u { plate(&p,&title,5.0*u,Color32::from_rgb(28,13,7),true); }
    let prompt=Rect::from_min_max(pos2(g.launch.left()-10.0*u,b.top()+10.0*u),pos2(b.right()+10.0*u,b.bottom()));
    plate(&p,&chamfer(prompt,[20.0*u,20.0*u,20.0*u,20.0*u]),4.0*u,Color32::from_rgb(28,13,7),true);
    let side_glass=Rect::from_min_max(pos2(interior.right()+9.0*u,interior.top()+c*0.5),
        pos2(f.right()-9.0*u,interior.top()+interior.height()*0.66));
    if side_glass.is_positive() { plate(&p,&chamfer(side_glass,[0.0,8.0*u,8.0*u,0.0]),1.7*u,Color32::from_rgb(53,23,8),false); }
    for at in [pos2(x+w-c*0.7,top+c*0.55),pos2(x+w-15.0*u,interior.top()+c*0.3),
        pos2(x+w-15.0*u,interior.bottom()-c*0.5),pos2(x+14.0*u,y+h-c*0.75)] { rivet(&p,at,5.0*u); }
}

fn rivet(p: &Painter, at: Pos2, r: f32) {
    p.circle_filled(at,r*1.5,Color32::from_rgb(20,7,2));
    p.circle_stroke(at,r*1.5,Stroke::new((r*0.25).max(0.5),Color32::from_rgb(147,72,20)));
    p.circle_filled(at,r,Color32::from_rgb(5,4,3));
    p.circle_filled(at+vec2(-0.25,-0.3)*r,r*0.28,Color32::from_rgb(133,112,88));
}

pub(super) fn emblem(p: &Painter, g: &Geometry, pose: BootPose, logo: TextureId) {
    if pose.emblem<=0.001 { return; }
    let seat=g.emblem.center();
    let start=pos2(g.viewport.center().x,g.viewport.top()+g.viewport.height()*0.43);
    let at=start+(seat-start)*pose.fly;
    let start_r=g.viewport.height().min(g.viewport.width())*0.145;
    let radius=(start_r+(g.emblem.width()*0.5-start_r)*pose.fly)*pose.emblem;
    let mut mesh=Mesh::default();
    let bands=[(0.0,Color32::from_rgb(68,17,2)),(0.74,Color32::from_rgb(25,7,2)),
        (0.82,Color32::from_rgb(16,5,2)),(0.86,Color32::from_rgb(235,105,29)),
        (0.91,Color32::from_rgb(255,192,103)),(0.97,Color32::from_rgb(123,49,11)),
        (1.04,Color32::from_rgb(244,117,36)),(1.08,Color32::from_rgb(65,22,5))];
    for band in 0..bands.len()-1 {
        for i in 0..96 {
            let a=i as f32/96.0*std::f32::consts::TAU;
            let b=(i+1) as f32/96.0*std::f32::consts::TAU;
            let base=mesh.vertices.len() as u32;
            for (angle,r,color) in [(a,bands[band].0,bands[band].1),(b,bands[band].0,bands[band].1),
                (b,bands[band+1].0,bands[band+1].1),(a,bands[band+1].0,bands[band+1].1)] {
                let light=(0.78+(-angle.sin()-angle.cos())*0.13).clamp(0.5,1.0);
                mesh.colored_vertex(at+vec2(angle.cos(),angle.sin())*r*radius,color.gamma_multiply(light*pose.emblem));
            }
            mesh.add_triangle(base,base+1,base+2);mesh.add_triangle(base,base+2,base+3);
        }
    }
    p.add(Shape::mesh(mesh));
    for angle in [-160.0_f32,-20.0,50.0,130.0] {
        let a=angle.to_radians(); let n=vec2(a.cos(),a.sin());let t=vec2(-n.y,n.x);
        let center=at+n*radius*1.08;
        let pts=[center-n*radius*0.10-t*radius*0.07,center+n*radius*0.10-t*radius*0.07,
            center+n*radius*0.10+t*radius*0.07,center-n*radius*0.10+t*radius*0.07];
        plate(p,&pts,1.5*g.scale,Color32::from_rgb(68,24,7),false);
    }
    let glint: Vec<_>=(0..=36).map(|i| {
        let a=std::f32::consts::PI+(i as f32/36.0)*1.9;
        at+vec2(a.cos(),a.sin())*radius*0.98
    }).collect();
    p.add(Shape::line(glint,Stroke::new(3.6*g.scale,LIGHT.gamma_multiply(0.7*pose.emblem))));
    image(p,Rect::from_center_size(at,vec2(radius*1.45,radius*0.625)),0.0,logo,
        Rect::from_min_max(Pos2::ZERO,pos2(1.0,1.0)),Color32::WHITE.gamma_multiply(pose.emblem));
}

pub(super) fn image(p: &Painter, rect: Rect, cut: f32, texture: TextureId, uv: Rect, tint: Color32) {
    let points=chamfer(rect,[cut;4]);
    let mut mesh=Mesh::with_texture(texture);
    mesh.vertices.push(egui::epaint::Vertex { pos: rect.center(),uv:uv.center(),color:tint });
    for v in points.iter() {
        let factor=(*v-rect.min)/rect.size();
        mesh.vertices.push(egui::epaint::Vertex { pos:*v,
            uv:pos2(uv.left()+factor.x*uv.width(),uv.top()+factor.y*uv.height()),color:tint });
    }
    for i in 0..8 { mesh.add_triangle(0,i+1,(i+1)%8+1); }
    p.add(Shape::mesh(mesh));
}
