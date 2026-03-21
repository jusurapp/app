use std::path::Path;
use ffmpeg_next as ffmpeg;

pub fn convert_to_wav_16k(input: &Path, output: &Path) -> Result<(), String> {
    ffmpeg::init().map_err(|e| format!("ffmpeg init: {e}"))?;

    // Open input
    let mut ictx = ffmpeg::format::input(input)
        .map_err(|e| format!("Failed to open audio input: {e}"))?;

    let audio_stream = ictx.streams().best(ffmpeg::media::Type::Audio)
        .ok_or("No audio stream found")?;
    let audio_idx = audio_stream.index();

    let decoder_ctx = ffmpeg::codec::context::Context::from_parameters(audio_stream.parameters())
        .map_err(|e| format!("Decoder context: {e}"))?;
    let mut decoder = decoder_ctx.decoder().audio()
        .map_err(|e| format!("Audio decoder: {e}"))?;

    // Resampler: input format -> 16kHz mono S16
    let mut resampler = ffmpeg::software::resampling::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed),
        ffmpeg::ChannelLayout::MONO,
        16000,
    ).map_err(|e| format!("Resampler: {e}"))?;

    // Open output (WAV)
    let mut octx = ffmpeg::format::output(output)
        .map_err(|e| format!("Failed to create WAV output: {e}"))?;

    let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::PCM_S16LE)
        .ok_or("PCM_S16LE encoder not found")?;

    let global_header = octx.format().flags().contains(ffmpeg::format::Flags::GLOBAL_HEADER);

    let mut ost = octx.add_stream(codec)
        .map_err(|e| format!("Failed to add output stream: {e}"))?;

    let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder().audio()
        .map_err(|e| format!("Encoder context: {e}"))?;

    encoder.set_rate(16000);
    encoder.set_channel_layout(ffmpeg::ChannelLayout::MONO);
    encoder.set_format(ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed));
    encoder.set_time_base(ffmpeg::Rational::new(1, 16000));

    if global_header {
        encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }

    let mut encoder = encoder.open_as(codec)
        .map_err(|e| format!("Failed to open encoder: {e}"))?;

    ost.set_parameters(&encoder);

    octx.write_header()
        .map_err(|e| format!("Failed to write header: {e}"))?;

    let mut decoded = ffmpeg::frame::Audio::empty();

    // Decode, resample, encode
    for (stream, packet) in ictx.packets() {
        if stream.index() != audio_idx { continue; }

        decoder.send_packet(&packet).map_err(|e| format!("Decoder send: {e}"))?;
        drain_decoder(&mut decoder, &mut resampler, &mut encoder, &mut octx, &mut decoded)?;
    }

    // Flush decoder
    decoder.send_eof().ok();
    drain_decoder(&mut decoder, &mut resampler, &mut encoder, &mut octx, &mut decoded)?;

    // Flush encoder
    encoder.send_eof().ok();
    drain_encoder(&mut encoder, &mut octx)?;

    octx.write_trailer()
        .map_err(|e| format!("Failed to write trailer: {e}"))?;

    Ok(())
}

fn drain_decoder(
    decoder: &mut ffmpeg::decoder::Audio,
    resampler: &mut ffmpeg::software::resampling::Context,
    encoder: &mut ffmpeg::encoder::Audio,
    octx: &mut ffmpeg::format::context::Output,
    decoded: &mut ffmpeg::frame::Audio,
) -> Result<(), String> {
    while decoder.receive_frame(decoded).is_ok() {
        let mut resampled = ffmpeg::frame::Audio::empty();
        resampler.run(decoded, &mut resampled)
            .map_err(|e| format!("Resampling failed: {e}"))?;

        encoder.send_frame(&resampled)
            .map_err(|e| format!("Encoder send: {e}"))?;
        drain_encoder(encoder, octx)?;
    }
    Ok(())
}

fn drain_encoder(
    encoder: &mut ffmpeg::encoder::Audio,
    octx: &mut ffmpeg::format::context::Output,
) -> Result<(), String> {
    let mut encoded = ffmpeg::Packet::empty();
    while encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(0);
        encoded.write_interleaved(octx)
            .map_err(|e| format!("Write packet: {e}"))?;
    }
    Ok(())
}
