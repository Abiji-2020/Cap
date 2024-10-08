use std::path::PathBuf;

use ffmpeg::{self as ffmpeg, Dictionary};

macro_rules! dict {
	( $($key:expr => $value:expr),* $(,)*) => ({
			let mut dict = ffmpeg::Dictionary::new();

			$(
				dict.set($key, $value);
			)*

			dict
		}
	);
}

pub struct H264Encoder {
    pub output: ffmpeg::format::context::Output,
    pub context: ffmpeg::encoder::Video,
    pub stream_index: usize,
    pub fps: f64,
    pub start_time: Option<u64>,
    last_frame: Option<ffmpeg::util::frame::Video>,
}

impl H264Encoder {
    pub fn output_format() -> ffmpeg::format::Pixel {
        ffmpeg::format::Pixel::YUV420P
    }

    pub fn new(path: &PathBuf, width: u32, height: u32, fps: f64) -> Self {
        let mut output = ffmpeg::format::output(path).unwrap();
        let output_flags = output.format().flags();

        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::H264).unwrap();

        let mut stream = output.add_stream(codec).unwrap();
        let stream_index = stream.index();

        let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .unwrap();

        stream.set_parameters(&encoder);
        encoder.set_width(width);
        encoder.set_height(height);
        encoder.set_format(Self::output_format());
        encoder.set_frame_rate(Some(fps));
        encoder.set_time_base(1.0 / fps);
        encoder.set_gop(fps as u32);
        encoder.set_max_bit_rate(8000);

        if output_flags.contains(ffmpeg::format::Flags::GLOBAL_HEADER) {
            encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }

        let encoder = encoder
            .open_as_with(
                codec,
                dict!("preset" => "ultrafast", "tune" => "zerolatency"),
            )
            .unwrap();
        stream.set_parameters(&encoder);
        stream.set_time_base(1.0 / fps);

        stream.set_metadata(Dictionary::from_iter(vec![("tune", "zerolatency")]));

        output.write_header().unwrap();

        Self {
            output,
            context: encoder,
            stream_index,
            start_time: None,
            fps,
            last_frame: None,
        }
    }

    pub fn encode_frame(&mut self, mut frame: ffmpeg::util::frame::Video, timestamp: u64) {
        let last_frame_pts = self.last_frame.as_ref().and_then(|f| f.pts());

        if let Some(last_frame_pts) = last_frame_pts {
            let pts = {
                let delta_time = if let Some(start_time) = self.start_time {
                    (timestamp - start_time) as i64
                } else {
                    self.start_time = Some(timestamp);

                    0
                };

                (delta_time as f64 / (1000.0 / self.fps)).round() as i64
            };

            // we should probably do something better than just dropping frames lol
            if pts <= last_frame_pts {
                return;
            }

            frame.set_pts(Some(pts));
        } else {
            frame.set_pts(Some(0));
        }

        self.context.send_frame(&frame).unwrap();
        self.last_frame = Some(frame);

        self.receive_and_process_packets();
    }

    fn receive_and_process_packets(&mut self) {
        let mut encoded = ffmpeg::Packet::empty();
        while self.context.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(self.stream_index);
            encoded.rescale_ts(
                1.0 / self.fps,
                self.output.stream(self.stream_index).unwrap().time_base(),
            );

            // println!(
            //     "writing packet - dts: {:?}, pts: {:?}, flags: {:?}",
            //     encoded.dts(),
            //     encoded.pts(),
            //     encoded.flags()
            // );

            encoded.write_interleaved(&mut self.output).unwrap();
        }
    }

    pub fn close(mut self) {
        self.context.send_eof().unwrap();

        self.receive_and_process_packets();

        self.output.write_trailer().unwrap();
    }
}

pub fn uyvy422_frame(bytes: &[u8], width: u32, height: u32) -> ffmpeg::frame::Video {
    let mut frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::UYVY422, width, height);

    frame.data_mut(0).copy_from_slice(bytes);

    frame
}

pub fn yuyv422_frame(bytes: &[u8], width: u32, height: u32) -> ffmpeg::frame::Video {
    let mut frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::YUYV422, width, height);

    frame.data_mut(0).copy_from_slice(bytes);

    frame
}

pub fn nv12_frame(bytes: &[u8], width: u32, height: u32) -> ffmpeg::frame::Video {
    let mut frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::NV12, width, height);
    let y_size = (width * height) as usize;
    let uv_size = y_size / 2;

    frame.data_mut(0)[..y_size].copy_from_slice(&bytes[..y_size]);
    frame.data_mut(1)[..uv_size].copy_from_slice(&bytes[y_size..y_size + uv_size]);

    frame
}

pub fn bgra_frame(bytes: &[u8], width: u32, height: u32) -> Option<ffmpeg::frame::Video> {
    let expected_size = (width * height * 4) as usize;

    if bytes.len() != expected_size {
        return None;
    }

    let mut frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::BGRA, width, height);
    frame.data_mut(0).copy_from_slice(bytes);

    Some(frame)
}

pub struct MP3Encoder {
    pub output: ffmpeg::format::context::Output,
    pub context: ffmpeg::encoder::Audio,
    pub stream_index: usize,
    pub sample_rate: u32,
    sample_number: i64,
}

impl MP3Encoder {
    pub fn new(path: &PathBuf, sample_rate: u32) -> Self {
        dbg!(sample_rate);
        let mut output = ffmpeg::format::output(path).unwrap();
        let output_flags = output.format().flags();

        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::MP3).unwrap();
        let audio_codec = codec.audio().unwrap();

        let mut stream = output.add_stream(audio_codec).unwrap();
        let stream_index = stream.index();

        let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .unwrap();

        stream.set_parameters(&encoder);
        encoder.set_rate(sample_rate as i32);
        encoder.set_bit_rate(128000);
        encoder.set_max_bit_rate(128000);
        encoder.set_channel_layout(ffmpeg::ChannelLayout::MONO);
        encoder.set_time_base((1, sample_rate as i32));
        encoder.set_format(ffmpeg::format::Sample::F32(
            ffmpeg::format::sample::Type::Packed,
        ));

        if output_flags.contains(ffmpeg::format::Flags::GLOBAL_HEADER) {
            encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }

        let encoder = encoder.open_as(codec).unwrap();
        stream.set_parameters(&encoder);
        stream.set_time_base((1, sample_rate as i32));

        Self {
            output,
            context: encoder,
            stream_index,
            sample_rate,
            sample_number: 0,
        }
    }

    pub fn encode_frame(&mut self, mut frame: ffmpeg::frame::Audio) {
        frame.set_pts(Some(self.sample_number as i64));
        self.context.send_frame(&frame).unwrap();
        self.sample_number += frame.samples() as i64;

        self.receive_and_process_packets();
    }

    fn receive_and_process_packets(&mut self) {
        let mut encoded = ffmpeg::Packet::empty();
        while self.context.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(self.stream_index);
            encoded.set_time_base(self.output.stream(self.stream_index).unwrap().time_base());

            encoded.write(&mut self.output).unwrap();
        }
    }
}
