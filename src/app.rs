use clap::{Parser, Subcommand, ValueEnum};
use image::{imageops::FilterType, DynamicImage};
use log::{debug, info};
use std::{fmt::Display, io::Write, path::PathBuf};

#[derive(Debug, Default, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ColorType {
    /// 3 bytes per pixel
    Rgb8,
    /// 2 bytes per color part. 6 bytes per pixel
    Rgb16,
    /// 1 bytes per pixel
    #[default]
    Gray8,
    /// 1 bit per pixel
    WB1,
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
    Bin,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
enum OutLang {
    #[default]
    C,
    Rust,
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
    #[arg(long, default_value = "__IMAGE", help = "define protect for C header")]
    protect: Option<String>,

    #[arg(long, default_value = "<cstdint>", help = "Include libs for C header")]
    include_c: Vec<String>,

    #[arg(long, default_value = "c")]
    out_lang: OutLang,

    #[arg(long, default_value = "IMAGE", help = "Name of const variable")]
    variable_name: String,

    #[arg(long, default_value = "false", help = "Inverse colors")]
    inverse_color: bool,

    #[arg(long, help = "Blur image")]
    blur: Option<f32>,

    #[arg(long, help = "Black level for wb1 out-color.", default_value = "128")]
    black_level: u8,
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
        let (step, img_buffer, width_del) = match self.args.out_color {
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
            ColorType::WB1 => (
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
        };
        let mut fout = std::fs::File::create(&self.args.output)?;

        if self.args.out_lang == OutLang::C {
            if let Some(p) = &self.args.protect {
                writeln!(fout, "#ifndef {}", p)?;
                writeln!(fout, "#define {}\n", p)?;
            }

            for include in &self.args.include_c {
                writeln!(fout, "#include {}", include)?;
            }
        }

        self.write_const(&mut fout, "IMAGE_HEIGHT", image.height() as usize)?;
        self.write_const(&mut fout, "IMAGE_WIDTH", image.width() as usize)?;
        self.write_const(&mut fout, "WIDTH_DELIMITER", width_del as usize)?;
        self.write_const_type(
            &mut fout,
            "IMAGE_WIDTH_BYTES",
            "IMAGE_WIDTH / WIDTH_DELIMITER",
            "usize",
        )?;

        self.write_const(&mut fout, "PIXEL_SIZE", step)?;
        self.write_const_type(
            &mut fout,
            "IMAGE_LENGTH",
            "IMAGE_WIDTH_BYTES * IMAGE_HEIGHT * PIXEL_SIZE",
            "usize",
        )?;

        match self.args.out_lang {
            OutLang::C => writeln!(
                fout,
                "uint8_t {}[IMAGE_LENGTH] = {{",
                self.args.variable_name
            )?,
            OutLang::Rust => writeln!(
                fout,
                "pub const {}: [u8; IMAGE_LENGTH] = [",
                self.args.variable_name
            )?,
        }

        for (i, p) in img_buffer.chunks(step).enumerate() {
            for p in p {
                match self.args.output_view {
                    OutputPreview::Hex => write!(fout, "0x{:02x}, ", p)?,
                    OutputPreview::Dec => write!(fout, "{:3}, ", p)?,
                    OutputPreview::Bin => write!(fout, "0b{:08b}, ", p)?,
                }
            }
            if (i > 0) && ((i + 1) as u32 % (image.width() / width_del) == 0) {
                debug!("New line on index: {}", i);
                writeln!(fout)?;
            }
        }
        match self.args.out_lang {
            OutLang::C => writeln!(fout, "}};")?,
            OutLang::Rust => writeln!(fout, "];")?,
        }

        if self.args.out_lang == OutLang::C {
            if let Some(p) = &self.args.protect {
                writeln!(fout, "#endif //{}", p)?;
            }
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
}
