use clap::{Parser, Subcommand, ValueEnum};
use image::{imageops::FilterType, DynamicImage};
use log::{debug, info};
use std::{fmt::Display, io::Write, path::PathBuf};

#[derive(Debug, Default, Clone, Copy, ValueEnum)]
enum ColorType {
    /// 3 bytes per pixel
    Rgb8,
    /// 2 bytes per color part. 6 bytes per pixel
    Rgb16,
    /// 1 bytes per pixel
    #[default]
    Gray8,
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
    #[arg(long, default_value = "IMAGE", help="define protect for C header")]
    protect: Option<String>,

    #[arg(long, default_value = "<cstdint>", help="Include libs for C header")]
    include_c: Vec<String>,

    #[arg(long, default_value = "c")]
    out_lang: OutLang,

    #[arg(long, default_value = "image", help="Name of const variable")]
    variable_name: String,

    #[arg(long, default_value = "false", help="Inverse colors")]
    inverse_color: bool,

    #[arg(long, help="Blur image")]
    blur: Option<f32>,
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
        let (step, img_buffer) = match self.args.out_color {
            ColorType::Rgb8 => (3, image.to_rgb8().into_vec()),
            ColorType::Rgb16 => (
                3 * 2,
                image
                    .to_rgb16()
                    .into_vec()
                    .into_iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
            ColorType::Gray8 => (1, image.to_luma8().into_vec()),
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

        self.write_const(&mut fout, "IMAGE_HEIGHT", image.height())?;
        self.write_const(&mut fout, "IMAGE_WIDTH", image.width())?;
        self.write_const(&mut fout, "PIXEL_SIZE", step as u32)?;
        self.write_const_type(
            &mut fout,
            "IMAGE_LENGTH",
            "IMAGE_WIDTH * IMAGE_HEIGHT * PIXEL_SIZE",
            "u32",
        )?;

        match self.args.out_lang {
            OutLang::C => writeln!(
                fout,
                "uint8_t {}[IMAGE_LENGTH] = {{",
                self.args.variable_name
            )?,
            OutLang::Rust => writeln!(
                fout,
                "const {}: [u8; IMAGE_LENGTH] = [",
                self.args.variable_name
            )?,
        }

        for (i, p) in img_buffer.chunks(step).enumerate() {
            for p in p {
                match self.args.output_view {
                    OutputPreview::Hex => write!(fout, "0x{:02x}, ", p)?,
                    OutputPreview::Dec => write!(fout, "{:3}, ", p)?,
                }
            }
            if (i > 0) && ((i + 1) as u32 % image.width() == 0) {
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
        name: &'static str,
        value: V,
    ) -> anyhow::Result<()> {
        self.write_const_type(fout, name, value, std::any::type_name::<V>())
    }

    fn write_const_type<V: Display>(
        &self,
        fout: &mut std::fs::File,
        name: &'static str,
        value: V,
        tp: &'static str,
    ) -> anyhow::Result<()> {
        match self.args.out_lang {
            OutLang::C => {
                writeln!(fout, "#define {} {}", name, value)?;
            }
            OutLang::Rust => {
                writeln!(fout, "const {}:{} = {};", name, tp, value)?;
            }
        }
        Ok(())
    }
}
