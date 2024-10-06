use std::path::PathBuf;

use clap::Parser;
use color_stuff::representations::CIEXYZCoords;
use exr::{
    image::{Encoding, Image, Layer, SpecificChannels},
    math::Vec2,
    meta::attribute::Chromaticities,
    prelude::{IntegerBounds, LayerAttributes, WritableImage},
};
use itertools::Itertools;
use nalgebra::SMatrix;
use rawloader::{decode_file, RawImageData};

type Matrix3x3f = SMatrix<f32, 3, 3>;
type Matrix3x1f = SMatrix<f32, 3, 1>;

// TODO: How should black and white levels be treated here ? Offset ? Linear map ?

#[derive(Parser)]
struct App {
    /// Path to camera raw file
    raw: PathBuf,
    /// Path to output OpenEXR file
    exr: PathBuf,
}

fn main() {
    let args = App::parse();

    let image = decode_file(args.raw).unwrap();

    let red_max = image.whitelevels[0] as f32;
    let green_max = image.whitelevels[1] as f32;
    let blue_max = image.whitelevels[2] as f32;

    // Throwing out last component, don't know what it's for really
    let m = image.cam_to_xyz();
    let cam_to_xyz = Matrix3x3f::new(
        m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2],
    );

    let mut red = Vec::with_capacity(image.width * image.height);
    let mut green = Vec::with_capacity(image.width * image.height);
    let mut blue = Vec::with_capacity(image.width * image.height);

    // Demosaicing is a damn headache
    if let RawImageData::Integer(data) = image.data {
        for (index, (y, x)) in (0..image.height)
            .cartesian_product(0..image.width)
            .enumerate()
        {
            // Load adjacent pixels
            let mut adjacent_pixels = [None; 8];

            // Check individually if pixels exist before adding them
            // Top Left
            if (x != 0) & (y != 0) {
                // Not in top left corner of image
                adjacent_pixels[0] = Some((data[index - image.width - 1] as f32, (x - 1, y - 1)));
            }

            // Top Middle
            if y != 0 {
                // Not on topmost row
                adjacent_pixels[1] = Some((data[index - image.width] as f32, (x, y - 1)));
            }

            // Top Right
            if (x != image.width - 1) & (y != 0) {
                // Not in top right corner of image
                adjacent_pixels[2] = Some((data[index - image.width + 1] as f32, (x + 1, y - 1)));
            }

            // Left
            if x != 0 {
                // Not on leftmost column
                adjacent_pixels[3] = Some((data[index - 1] as f32, (x - 1, y)));
            }

            // Right
            if x != image.width - 1 {
                // Not on rightmost column
                adjacent_pixels[4] = Some((data[index + 1] as f32, (x + 1, y)));
            }

            // Bottom Left
            if (x != 0) & (y != image.height - 1) {
                // Not on bottom left
                adjacent_pixels[5] = Some((data[index + image.width - 1] as f32, (x - 1, y + 1)));
            }

            // Bottom Middle
            if y != image.height - 1 {
                // Not on bottommost row
                adjacent_pixels[6] = Some((data[index + image.width] as f32, (x, y + 1)));
            }

            // Bottom Right
            if (x != image.width - 1) & (y != image.height - 1) {
                // Not on bottom left
                adjacent_pixels[7] = Some((data[index + image.width + 1] as f32, (x + 1, y + 1)));
            }

            // Pixel sums
            let mut red_sum = 0.0;
            let mut green_sum = 0.0;
            let mut blue_sum = 0.0;

            // Pixel counts
            let mut red_count = 0;
            let mut green_count = 0;
            let mut blue_count = 0;

            for (value, (x, y)) in adjacent_pixels.into_iter().flatten() {
                match image.cfa.color_at(y, x) {
                    0 => {
                        // Red
                        red_sum += value;
                        red_count += 1;
                    }
                    1 => {
                        // Green
                        green_sum += value;
                        green_count += 1;
                    }
                    2 => {
                        // Blue
                        blue_sum += value;
                        blue_count += 1;
                    }
                    _ => panic!(),
                }
            }

            // Average pixels
            let red_avg = red_sum / red_count as f32 / red_max;
            let green_avg = green_sum / green_count as f32 / green_max;
            let blue_avg = blue_sum / blue_count as f32 / blue_max;

            // Select outputs
            let red_output;
            let green_output;
            let blue_output;

            // Use direct component
            match image.cfa.color_at(y, x) {
                0 => {
                    // Red
                    red_output = data[index] as f32 / red_max;
                    green_output = green_avg;
                    blue_output = blue_avg;
                }
                1 => {
                    // Green
                    red_output = red_avg;
                    green_output = data[index] as f32 / green_max;
                    blue_output = blue_avg;
                }
                2 => {
                    // Blue
                    red_output = red_avg;
                    green_output = green_avg;
                    blue_output = data[index] as f32 / blue_max;
                }
                _ => panic!(),
            }

            red.push(red_output);
            green.push(green_output);
            blue.push(blue_output);
        }
    } else {
        unimplemented!()
    }

    // Convert CAM to XYZ matrix into chromaticities by "probing" colors
    let red_point = Matrix3x1f::new(red_max, 0.0, 0.0);
    let green_point = Matrix3x1f::new(0.0, green_max, 0.0);
    let blue_point = Matrix3x1f::new(0.0, 0.0, blue_max);
    let white_point = Matrix3x1f::new(red_max, green_max, blue_max);

    // These conversions shouldn't fail unless provided info is wrong
    let red_xyy = CIEXYZCoords::from(cam_to_xyz * red_point)
        .try_xyy()
        .unwrap();
    let green_xyy = CIEXYZCoords::from(cam_to_xyz * green_point)
        .try_xyy()
        .unwrap();
    let blue_xyy = CIEXYZCoords::from(cam_to_xyz * blue_point)
        .try_xyy()
        .unwrap();
    let white_xyy = CIEXYZCoords::from(cam_to_xyz * white_point)
        .try_xyy()
        .unwrap();

    let pixels_fn = |pos: Vec2<usize>| {
        (
            red[image.width * pos.y() + pos.x()],
            green[image.width * pos.y() + pos.x()],
            blue[image.width * pos.y() + pos.x()],
        )
    };

    let layer = Layer::new(
        (image.width, image.height),
        LayerAttributes::named("RAW Image"),
        Encoding::SMALL_FAST_LOSSLESS,
        SpecificChannels::rgb(pixels_fn),
    );

    let mut exr_image = Image::from_layer(layer);
    exr_image.attributes.pixel_aspect = 1.0;
    exr_image.attributes.display_window =
        crops_size_to_bounds(image.crops, image.width, image.height);
    exr_image.attributes.chromaticities = Some(Chromaticities {
        red: red_xyy.coords.into(),
        green: green_xyy.coords.into(),
        blue: blue_xyy.coords.into(),
        white: white_xyy.coords.into(),
    });

    exr_image.write().to_file(args.exr).unwrap();
}

fn crops_size_to_bounds(crops: [usize; 4], width: usize, height: usize) -> IntegerBounds {
    let top = crops[0];
    let right = crops[1];
    let bottom = crops[2];
    let left = crops[3];
    IntegerBounds {
        position: Vec2(left as i32, top as i32),
        size: Vec2(width - left - right, height - top - bottom),
    }
}
