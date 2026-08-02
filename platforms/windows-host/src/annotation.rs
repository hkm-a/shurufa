//! 截图标注的不可变对象与撤销历史。
//!
//! 模型遵循 Flameshot 的 `ModificationCommand` 思路：底图始终保持原样，
//! 每次用户操作只追加一个标注对象；撤销与重做只移动对象，不重新采集屏幕。

/// 以截图左上角为原点的像素坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// RGBA 标注颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Color {
    pub const RED: Self = Self {
        red: 245,
        green: 76,
        blue: 76,
        alpha: 255,
    };
}

/// 一个不可变标注对象。坐标统一相对于裁剪后的截图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Annotation {
    Rectangle {
        start: Point,
        end: Point,
        color: Color,
        stroke_width: u16,
    },
    Arrow {
        start: Point,
        end: Point,
        color: Color,
        stroke_width: u16,
    },
    Mosaic {
        start: Point,
        end: Point,
        block_size: u16,
    },
    Text {
        origin: Point,
        content: String,
        color: Color,
        font_size: u16,
    },
}

/// 标注会话的命令栈。只有 `add` 会改变对象列表；`undo` 与 `redo` 始终成对移动。
#[derive(Debug, Default)]
pub struct EditorState {
    annotations: Vec<Annotation>,
    redo_stack: Vec<Annotation>,
}

impl EditorState {
    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn can_undo(&self) -> bool {
        !self.annotations.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// 新操作会丢弃历史分支，保证撤销/重做符合常见编辑器预期。
    pub fn add(&mut self, annotation: Annotation) {
        self.annotations.push(annotation);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(annotation) = self.annotations.pop() else {
            return false;
        };
        self.redo_stack.push(annotation);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(annotation) = self.redo_stack.pop() else {
            return false;
        };
        self.annotations.push(annotation);
        true
    }
}

/// 把已确认的标注渲染到导出副本。底图参数永不修改，调用方可用它重绘预览。
pub fn render(base: &image::RgbaImage, annotations: &[Annotation]) -> image::RgbaImage {
    let mut output = base.clone();
    for annotation in annotations {
        match annotation {
            Annotation::Rectangle {
                start,
                end,
                color,
                stroke_width,
            } => draw_rectangle(&mut output, *start, *end, *color, *stroke_width),
            Annotation::Arrow {
                start,
                end,
                color,
                stroke_width,
            } => draw_arrow(&mut output, *start, *end, *color, *stroke_width),
            Annotation::Mosaic {
                start,
                end,
                block_size,
            } => draw_mosaic(&mut output, *start, *end, *block_size),
            // 中文字体字形由 Windows GDI 在编辑器中渲染；这里保留文本对象，
            // 使无字体的纯像素路径仍可用于形状预览与单元测试。
            Annotation::Text { .. } => {}
        }
    }
    output
}

fn draw_rectangle(
    image: &mut image::RgbaImage,
    start: Point,
    end: Point,
    color: Color,
    width: u16,
) {
    let left = start.x.min(end.x);
    let right = start.x.max(end.x);
    let top = start.y.min(end.y);
    let bottom = start.y.max(end.y);
    draw_line(
        image,
        Point { x: left, y: top },
        Point { x: right, y: top },
        color,
        width,
    );
    draw_line(
        image,
        Point { x: right, y: top },
        Point {
            x: right,
            y: bottom,
        },
        color,
        width,
    );
    draw_line(
        image,
        Point {
            x: right,
            y: bottom,
        },
        Point { x: left, y: bottom },
        color,
        width,
    );
    draw_line(
        image,
        Point { x: left, y: bottom },
        Point { x: left, y: top },
        color,
        width,
    );
}

fn draw_arrow(image: &mut image::RgbaImage, start: Point, end: Point, color: Color, width: u16) {
    draw_line(image, start, end, color, width);
    let dx = (end.x - start.x) as f64;
    let dy = (end.y - start.y) as f64;
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1.0 {
        return;
    }
    let size = (width.max(2) as f64 * 3.5).clamp(8.0, 24.0);
    let unit_x = dx / length;
    let unit_y = dy / length;
    let perpendicular_x = -unit_y;
    let perpendicular_y = unit_x;
    for direction in [-1.0, 1.0] {
        let point = Point {
            x: (end.x as f64 - unit_x * size + perpendicular_x * size * 0.55 * direction).round()
                as i32,
            y: (end.y as f64 - unit_y * size + perpendicular_y * size * 0.55 * direction).round()
                as i32,
        };
        draw_line(image, end, point, color, width);
    }
}

fn draw_line(image: &mut image::RgbaImage, start: Point, end: Point, color: Color, width: u16) {
    let mut x = start.x;
    let mut y = start.y;
    let dx = (end.x - start.x).unsigned_abs() as i32;
    let sx = if start.x < end.x { 1 } else { -1 };
    let dy = -((end.y - start.y).unsigned_abs() as i32);
    let sy = if start.y < end.y { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        draw_dot(image, x, y, color, width);
        if x == end.x && y == end.y {
            break;
        }
        let doubled = error * 2;
        if doubled >= dy {
            error += dy;
            x += sx;
        }
        if doubled <= dx {
            error += dx;
            y += sy;
        }
    }
}

fn draw_dot(image: &mut image::RgbaImage, x: i32, y: i32, color: Color, width: u16) {
    let radius = i32::from(width.max(1) - 1) / 2;
    for offset_y in -radius..=radius {
        for offset_x in -radius..=radius {
            let pixel_x = x + offset_x;
            let pixel_y = y + offset_y;
            if pixel_x >= 0
                && pixel_y >= 0
                && pixel_x < image.width() as i32
                && pixel_y < image.height() as i32
            {
                image.put_pixel(
                    pixel_x as u32,
                    pixel_y as u32,
                    image::Rgba([color.red, color.green, color.blue, color.alpha]),
                );
            }
        }
    }
}

fn draw_mosaic(image: &mut image::RgbaImage, start: Point, end: Point, block_size: u16) {
    let left = start.x.min(end.x).max(0) as u32;
    let right = start.x.max(end.x).clamp(0, image.width() as i32) as u32;
    let top = start.y.min(end.y).max(0) as u32;
    let bottom = start.y.max(end.y).clamp(0, image.height() as i32) as u32;
    if left >= right || top >= bottom {
        return;
    }
    let size = u32::from(block_size.max(2));
    let source = image.clone();
    for block_top in (top..bottom).step_by(size as usize) {
        for block_left in (left..right).step_by(size as usize) {
            let block_right = (block_left + size).min(right);
            let block_bottom = (block_top + size).min(bottom);
            let mut total = [0u64; 4];
            let mut count = 0u64;
            for y in block_top..block_bottom {
                for x in block_left..block_right {
                    let pixel = source.get_pixel(x, y).0;
                    for (sum, component) in total.iter_mut().zip(pixel) {
                        *sum += u64::from(component);
                    }
                    count += 1;
                }
            }
            let color = image::Rgba(total.map(|value| (value / count) as u8));
            for y in block_top..block_bottom {
                for x in block_left..block_right {
                    image.put_pixel(x, y, color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle() -> Annotation {
        Annotation::Rectangle {
            start: Point { x: 4, y: 8 },
            end: Point { x: 40, y: 80 },
            color: Color::RED,
            stroke_width: 3,
        }
    }

    fn arrow() -> Annotation {
        Annotation::Arrow {
            start: Point { x: 10, y: 20 },
            end: Point { x: 50, y: 60 },
            color: Color::RED,
            stroke_width: 2,
        }
    }

    fn text() -> Annotation {
        Annotation::Text {
            origin: Point { x: 12, y: 16 },
            content: "中文标注".to_owned(),
            color: Color::RED,
            font_size: 20,
        }
    }

    #[test]
    fn 标注按照用户操作顺序保存() {
        let mut state = EditorState::default();
        state.add(rectangle());
        state.add(arrow());
        state.add(text());
        assert_eq!(state.annotations(), &[rectangle(), arrow(), text()]);
        assert!(state.can_undo());
        assert!(!state.can_redo());
    }

    #[test]
    fn 撤销和重做不会改变对象本身() {
        let mut state = EditorState::default();
        state.add(rectangle());
        assert!(state.undo());
        assert!(state.annotations().is_empty());
        assert!(state.can_redo());
        assert!(state.redo());
        assert_eq!(state.annotations(), &[rectangle()]);
    }

    #[test]
    fn 新操作会丢弃已撤销的分支() {
        let mut state = EditorState::default();
        state.add(rectangle());
        state.add(arrow());
        assert!(state.undo());
        state.add(Annotation::Mosaic {
            start: Point { x: 0, y: 0 },
            end: Point { x: 20, y: 20 },
            block_size: 8,
        });
        assert!(!state.can_redo());
        assert_eq!(state.annotations().len(), 2);
    }

    #[test]
    fn 空历史不能撤销或重做() {
        let mut state = EditorState::default();
        assert!(!state.undo());
        assert!(!state.redo());
    }

    #[test]
    fn 导出渲染不会修改冻结的底图() {
        let base = image::RgbaImage::from_pixel(32, 32, image::Rgba([10, 20, 30, 255]));
        let rendered = render(&base, &[rectangle()]);
        assert_eq!(base.get_pixel(4, 8).0, [10, 20, 30, 255]);
        assert_eq!(rendered.get_pixel(4, 8).0, [245, 76, 76, 255]);
    }

    #[test]
    fn 马赛克将块内像素归并为单色() {
        let mut base = image::RgbaImage::new(4, 4);
        for (index, pixel) in base.pixels_mut().enumerate() {
            *pixel = image::Rgba([index as u8, 0, 0, 255]);
        }
        let rendered = render(
            &base,
            &[Annotation::Mosaic {
                start: Point { x: 0, y: 0 },
                end: Point { x: 4, y: 4 },
                block_size: 4,
            }],
        );
        assert!(rendered.pixels().all(|pixel| pixel.0 == [7, 0, 0, 255]));
    }
}
