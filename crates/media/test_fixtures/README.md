These synthetic 256×192 H.264 High-profile MP4 fixtures were created for Fanta's native poster decoder tests. They contain one second at four frames per second. The first half has red/green/blue/yellow quadrants; the second half inverts their colors, making the first-frame check distinguish a different timestamp. There is no audio or external media reference. The encoded samples explicitly use limited-range BT.709 with matching matrix, primaries, and transfer tags.

The 90°, 180°, 270°, and mirrored files contain the same encoded samples with only the standard `tkhd` transform matrix changed. Their orientation and quadrant pixels were independently checked with ffprobe/ffmpeg. The corrupt fixture retains the original MP4 metadata but replaces the `mdat` payload with zeroes, so it must fail actual decoding.

`quadrants-tiny.mp4` is a separate 64×48 no-upscaling control encoded with `8x8dct=0`. On the validation Mac, AVFoundation and ffmpeg disagreed on the tiny clip's decoded chroma when 8×8 transforms were enabled; disabling only that encoder option corrected the native pixels. The larger High-profile fixture retains 8×8 transforms. Untagged samples also exercised a native fallback color profile, so explicit BT.709 tags avoid ambiguous color expectations. Test color tolerance is unchanged; these fixtures do not establish correctness for every platform/codec combination.

The primary fixture was generated with ffmpeg/libx264:

```sh
ffmpeg -f lavfi -i color=black:s=256x192:r=4:d=1 \
  -vf "drawbox=x=0:y=0:w=128:h=96:color=red:t=fill,drawbox=x=128:y=0:w=128:h=96:color=lime:t=fill,drawbox=x=0:y=96:w=128:h=96:color=blue:t=fill,drawbox=x=128:y=96:w=128:h=96:color=yellow:t=fill,scale=in_color_matrix=bt601:out_color_matrix=bt709,negate=enable='gte(t,0.5)'" \
  -c:v libx264 -crf 12 -pix_fmt yuv420p -movflags +faststart \
  -color_range tv -colorspace bt709 -color_primaries bt709 -color_trc bt709 quadrants.mp4
```

For the tiny control, dimensions and quadrant boundaries are divided by four, and `-x264-params 8x8dct=0` is added. The variants' transform matrices apply the corresponding quarter turn or horizontal reflection with positive display-space translation.

All fixture artwork and generated clips are original synthetic test data, released under the repository's Apache-2.0 license. Runtime tests use the platform decoder; ffmpeg is not a dependency of the app or its tests.
