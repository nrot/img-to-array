use clap::{Parser, Subcommand, ValueEnum};
use image::{imageops::FilterType, DynamicImage, GenericImageView, Pixel};
use log::{debug, info, warn};
use std::{ffi::OsStr, fmt::Display, io::Write, path::PathBuf};

#[derive(Debug, Default, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ColorType {
    /// 3 bytes per pixel
    Rgb8,
    /// 2 bytes per color part. 6 bytes per pixel
    Rgb16,
    /// 1 bytes per pixel
    #[default]
    Gray8,
    /// Decoded
    WBZip,
    /// 1 bit per pixel
    WB1,
    ///
    SSD1306,
    ///
    GCode,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum ResizeFilter {
    Nearest,
    Triangle,
    #[default]
    CatmullRom,
    Gaussian,
    Lanczos3,
}

impl From<ResizeFilter> for FilterType {
    fn from(val: ResizeFilter) -> Self {
        match val {
            ResizeFilter::Nearest => FilterType::Nearest,
            ResizeFilter::Triangle => FilterType::Triangle,
            ResizeFilter::CatmullRom => FilterType::CatmullRom,
            ResizeFilter::Gaussian => FilterType::Gaussian,
            ResizeFilter::Lanczos3 => FilterType::Lanczos3,
        }
    }
}

#[derive(Parser, Debug, Clone, Copy)]
struct ResizeParam {
    #[arg(long)]
    width: u32,
    #[arg(long)]
    height: u32,
    #[arg(long, default_value = "triangle")]
    filter: ResizeFilter,
}

#[derive(Subcommand, Debug, Clone, Copy)]
enum ResizeType {
    Resize(ResizeParam),
    ResizeExact(ResizeParam),
    ResizeFill(ResizeParam),
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum OutputPreview {
    #[default]
    Hex,
    Dec,
    SDec,
    Bin,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
enum OutLang {
    #[default]
    C,
    Rust,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
enum Ending {
    /// Litle ending
    #[default]
    Le,
    Be,
}

#[derive(Parser, Debug)]
struct Arg {
    #[arg(help = "Input image")]
    input: PathBuf,
    #[arg(help = "Output file")]
    output: PathBuf,
    #[arg(short, long, default_value = "gray8")]
    out_color: ColorType,
    #[clap(flatten)]
    verbose: clap_verbosity_flag::Verbosity,
    #[command(subcommand)]
    resize: Option<ResizeType>,
    #[arg(long, default_value = "hex")]
    output_view: OutputPreview,
    #[arg(long, help = "define protect for C header")]
    protect: Option<String>,

    #[arg(long, default_value = "<stdint.h>", help = "Include libs for C header")]
    include_c: Vec<String>,

    #[arg(long, default_value = "c")]
    out_lang: OutLang,

    #[arg(long, short, help = "Name of const variable")]
    name_variable: Option<String>,

    #[arg(long, short, default_value = "false", help = "Inverse colors")]
    inverse_color: bool,

    #[arg(long, help = "Blur image")]
    blur: Option<f32>,

    #[arg(long, help = "Black level for wb1 out-color.", default_value = "128")]
    black_level: u8,

    #[arg(long, help = "Ending out pixel", default_value = "le")]
    ending: Ending,
}

pub struct App {
    args: Arg,
}

impl App {
    pub fn new() -> Self {
        Self { args: Arg::parse() }
    }
    pub fn log_level_filter(&self) -> log::LevelFilter {
        self.args.verbose.log_level_filter()
    }

    pub fn work(&mut self) -> anyhow::Result<()> {
        let mut image = image::open(&self.args.input)?;
        let image_name = self.args.name_variable.clone().unwrap_or_else(|| {
            self.args
                .input
                .file_name()
                .unwrap_or_else(|| OsStr::new("IMAGE"))
                .to_str()
                .unwrap_or_else(|| "IMAGE")
                .to_uppercase()
                .replace('-', "_")
                .split('.')
                .take(1)
                .collect()
        });

        if self.args.inverse_color {
            image.invert();
        }

        if let Some(b) = self.args.blur {
            info!("Blur by {:.2}", b);
            image = image.blur(b);
        }

        if let Some(ni) = self.resize(&image) {
            image = ni;
        }
        let (step, mut img_buffer, width_del) = match self.args.out_color {
            ColorType::GCode => (1usize, image.to_luma8().into_vec(), 1),
            ColorType::Rgb8 => (3usize, image.to_rgb8().into_vec(), 1),
            ColorType::Rgb16 => (
                3 * 2,
                image
                    .to_rgb16()
                    .into_vec()
                    .into_iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
                1,
            ),
            ColorType::Gray8 => (1, image.to_luma8().into_vec(), 1),
            ColorType::WBZip => (
                1,
                image
                    .to_luma8()
                    .into_iter()
                    .map(|v| if *v > self.args.black_level { 255 } else { 0 })
                    .collect(),
                1,
            ),
            ColorType::WB1 | ColorType::SSD1306 => (
                1,
                image
                    .to_luma8()
                    .into_vec()
                    .chunks(8)
                    .map(|v| {
                        let mut re = 0u8;
                        v.iter().for_each(|b| {
                            re <<= 1;
                            if *b > self.args.black_level {
                                re |= 0b1;
                            };
                        });
                        re
                    })
                    .collect(),
                8,
            ),
            // ColorType::SSD1306 => (1, image.to_luma8().into_vec(), 8),
        };
        let mut fout = std::fs::File::create(&self.args.output)?;

        if self.args.out_lang == OutLang::C {
            let p = self.args.protect.as_ref().unwrap_or_else(|| &image_name);
            writeln!(fout, "#ifndef __{}", p)?;
            writeln!(fout, "#define __{}\n", p)?;

            for include in &self.args.include_c {
                writeln!(fout, "#include {}", include)?;
            }
        }

        self.write_const(
            &mut fout,
            &format!("{}_HEIGHT", image_name),
            image.height() as usize,
        )?;
        self.write_const(
            &mut fout,
            &format!("{}_WIDTH", image_name),
            image.width() as usize,
        )?;
        self.write_const(
            &mut fout,
            &format!("{}_WIDTH_DELIMITER", image_name),
            width_del as usize,
        )?;
        self.write_const_type(
            &mut fout,
            &format!("{}_WIDTH_BYTES", image_name),
            &format!("{0}_WIDTH / {0}_WIDTH_DELIMITER", image_name),
            "usize",
        )?;

        self.write_const(&mut fout, &format!("{}_PIXEL_SIZE", image_name), step)?;
        let image_length =
            image.width() as f32 * image.height() as f32 * step as f32 / width_del as f32;
        let image_lenght = if image_length - image_length.floor() > 0.0 {
            format!(
                "{0}_HEIGHT * {0}_PIXEL_SIZE * {0}_WIDTH_BYTES + 1",
                image_name
            )
        } else {
            format!("{0}_HEIGHT * {0}_PIXEL_SIZE * {0}_WIDTH_BYTES", image_name)
        };
        self.write_const_type(
            &mut fout,
            &format!("{}_LENGTH", image_name),
            &image_lenght,
            "usize",
        )?;

        match self.args.out_color {
            ColorType::WBZip => {}
            _ => match self.args.out_lang {
                OutLang::C => writeln!(fout, "uint8_t {}[{}_LENGTH] = {{", image_name, image_name)?,
                OutLang::Rust => writeln!(
                    fout,
                    "pub const {}: [u8; {}_LENGTH] = [",
                    image_name, image_name
                )?,
            },
        }

        match self.args.out_color {
            ColorType::WBZip => {
                let mut buff = Vec::new();
                let mut color = image.get_pixel(0, 0).to_luma().0[0] > self.args.black_level;
                let mut color_s = 0u8;
                let mut first = true;
                for y in 0..image.height() {
                    for x in 0..image.width() {
                        let c = image.get_pixel(x, y).to_luma().0[0] > self.args.black_level;
                        if color != c || color_s == 127 {
                            buff.push(if color {
                                0b10000000 | color_s
                            } else {
                                color_s
                            });
                            color = c;
                            color_s = 0;
                        } else {
                            if first{
                                first = false;
                            } else {
                                color_s += 1;
                            }
                        }
                    }
                    // buff.push(if color.to_luma().0[0] > self.args.black_level  {
                    //     0b10000000 | color_s
                    // } else {
                    //     color_s
                    // });
                    // color = image.get_pixel(0, (y + 1) % image.height());
                    // color_s = 0;
                }
                let len = buff.len() as u16;
                buff.insert(0, len.to_le_bytes()[0]);
                buff.insert(0, len.to_le_bytes()[1]);
                match self.args.out_lang {
                    OutLang::C => writeln!(fout, "uint8_t {}[{}] = {{", image_name, buff.len())?,
                    OutLang::Rust => {
                        writeln!(fout, "pub const {}: [u8; {}] = [", image_name, buff.len())?
                    }
                }
                for p in &buff {
                    match self.args.output_view {
                        OutputPreview::Hex => write!(fout, "0x{:02x}, ", self.to_ending(p))?,
                        OutputPreview::Dec => write!(fout, "{:3}, ", self.to_ending(p))?,
                        OutputPreview::SDec => write!(fout, "{:3}, ", *p as u16 as i8)?,
                        OutputPreview::Bin => write!(fout, "0b{:08b}, ", self.to_ending(p))?,
                    }
                }
            }
            ColorType::SSD1306 => {
                img_buffer.iter_mut().for_each(|v| {
                    *v = !self.args.inverse_color as u8;
                });
                for irb in 0..((image.height() as f32 / 8.0).ceil() as u32) {
                    for ic in 0..image.width() {
                        for cc in 0..8 {
                            if irb * 8 + cc >= image.height() {
                                continue;
                            }
                            let p = image.get_pixel(ic, irb * 8 + cc).to_luma();
                            if let Some(b) = img_buffer.get_mut((irb * image.width() + ic) as usize)
                            {
                                *b = *b & !(1 << cc)
                                    | (((p.0[0] > self.args.black_level) as u8) << cc);
                            } else {
                                warn!(
                                    "Outside image set pixel: {}, {}",
                                    ic,
                                    irb * image.width() + cc
                                );
                            };
                        }
                    }
                }
            }
            ColorType::GCode=>{
                for (i, p) in img_buffer.chunks(step).enumerate() {
                    todo!("тут");
                }
            }
            _ => {
                for (i, p) in img_buffer.chunks(step).enumerate() {
                    for p in p {
                        match self.args.output_view {
                            OutputPreview::Hex => write!(fout, "0x{:02x}, ", self.to_ending(p))?,
                            OutputPreview::Dec => write!(fout, "{:3}, ", self.to_ending(p))?,
                            OutputPreview::SDec => write!(fout, "{:3}, ", *p as u16 as i8)?,
                            OutputPreview::Bin => write!(fout, "0b{:08b}, ", self.to_ending(p))?,
                        }
                    }
                    if (i > 0) && ((i + 1) as u32 % (image.width() / width_del) == 0) {
                        debug!("New line on index: {}", i);
                        writeln!(fout)?;
                    }
                }
            }
        }

        match self.args.out_lang {
            OutLang::C => writeln!(fout, "}};")?,
            OutLang::Rust => writeln!(fout, "];")?,
        }

        if self.args.out_lang == OutLang::C {
            let p = self.args.protect.as_ref().unwrap_or_else(|| &image_name);
            writeln!(fout, "#endif //__{}", p)?;
        }

        fout.sync_data()?;
        Ok(())
    }

    fn resize(&self, image: &DynamicImage) -> Option<DynamicImage> {
        if let Some(resize) = self.args.resize {
            match resize {
                ResizeType::Resize(p) => Some(image.resize(p.width, p.height, p.filter.into())),
                ResizeType::ResizeExact(p) => {
                    Some(image.resize_exact(p.width, p.height, p.filter.into()))
                }
                ResizeType::ResizeFill(p) => {
                    Some(image.resize_to_fill(p.width, p.height, p.filter.into()))
                }
            }
        } else {
            None
        }
    }

    fn write_const<V: Display>(
        &self,
        fout: &mut std::fs::File,
        name: &str,
        value: V,
    ) -> anyhow::Result<()> {
        self.write_const_type(fout, name, value, std::any::type_name::<V>())
    }

    fn write_const_type<V: Display>(
        &self,
        fout: &mut std::fs::File,
        name: &str,
        value: V,
        tp: &str,
    ) -> anyhow::Result<()> {
        match self.args.out_lang {
            OutLang::C => {
                writeln!(fout, "#define {} {}", name, value)?;
            }
            OutLang::Rust => {
                writeln!(fout, "pub const {}:{} = {};", name, tp, value)?;
            }
        }
        Ok(())
    }

    fn to_ending<T: ToOrder>(&self, v: &T) -> u8 {
        match self.args.ending {
            Ending::Le => v.le(),
            Ending::Be => v.be(),
        }
    }
}

trait ToOrder {
    fn le(&self) -> u8;
    fn be(&self) -> u8;
}

impl ToOrder for u8 {
    fn le(&self) -> u8 {
        self.to_le()
    }

    fn be(&self) -> u8 {
        self.to_be()
    }
}

impl ToOrder for i8 {
    fn le(&self) -> u8 {
        self.to_le() as u8
    }

    fn be(&self) -> u8 {
        self.to_be() as u8
    }
}
