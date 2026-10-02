//! Test the actual export pixel stitching without a GPU.
use anyhow::Result;
include!(concat!(env!("OUT_DIR"), "/best_board_pixels.rs"));

#[test]
fn tiles_flip_vertically_and_last_partial_tile_uses_its_top_rows() {
    let tile = [10; 4].into_iter().chain([20; 4]).chain([30; 4]).collect::<Vec<_>>();
    let mut image = vec![0; 20];
    copy_tile(&mut image, &tile, 1, 3, 0, 3).unwrap();
    copy_tile(&mut image, &tile, 1, 3, 3, 2).unwrap();
    assert_eq!(
        image,
        [30; 4]
            .into_iter()
            .chain([20; 4])
            .chain([10; 4])
            .chain([30; 4])
            .chain([20; 4])
            .collect::<Vec<_>>()
    );
}
#[test]
fn invalid_tiles_fail_before_writing_the_image() {
    let mut image = vec![1; 8];
    assert!(copy_tile(&mut image, &[0; 8], 1, 2, 1, 2).is_err());
    assert!(copy_tile(&mut image, &[0; 4], 1, 2, 0, 1).is_err());
    assert!(copy_tile(&mut image, &[0; 8], 0, 2, 0, 1).is_err());
    assert_eq!(image, vec![1; 8]);
}
