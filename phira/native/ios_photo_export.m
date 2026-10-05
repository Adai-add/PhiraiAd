#import <Foundation/Foundation.h>
#import <Photos/Photos.h>
#import <dispatch/dispatch.h>
#include <stddef.h>
#include <stdint.h>

typedef void (*PhiraiAdPhotoCompletion)(void *context, int success, const char *error);

// Called by the PNG encoding worker. Copy data before returning to Rust;
// ARC blocks keep it alive through permission and asynchronous Photos changes.
void phiraiad_save_png_to_photos(const uint8_t *bytes, size_t length,
                                const char *filename,
                                PhiraiAdPhotoCompletion completion, void *context) {
    @autoreleasepool {
        @try {
            NSData *data = [NSData dataWithBytes:bytes length:length];
            NSString *name = [NSString stringWithUTF8String:filename];
            if (data.length == 0 || name == nil) {
                completion(context, 0, "图片数据无效");
                return;
            }
            dispatch_async(dispatch_get_main_queue(), ^{
                @try {
                    void (^authorized)(PHAuthorizationStatus) = ^(PHAuthorizationStatus status) {
                        @autoreleasepool {
                            if (status != PHAuthorizationStatusAuthorized && status != PHAuthorizationStatusLimited) {
                                completion(context, 0, "未获得相册写入权限，请在系统设置中允许 PhiraiAd 添加照片");
                                return;
                            }
                            @try {
                                __block NSString *changeError = nil;
                                [[PHPhotoLibrary sharedPhotoLibrary] performChanges:^{
                                    @try {
                                    PHAssetCreationRequest *request = [PHAssetCreationRequest creationRequestForAsset];
                                    PHAssetResourceCreationOptions *options = [PHAssetResourceCreationOptions new];
                                    options.originalFilename = name;
                                    options.uniformTypeIdentifier = @"public.png";
                                    [request addResourceWithType:PHAssetResourceTypePhoto data:data options:options];
                                    } @catch (NSException *exception) {
                                        changeError = exception.reason ?: @"相册资源创建异常";
                                    }
                                } completionHandler:^(BOOL success, NSError *error) {
                                    @autoreleasepool {
                                        BOOL saved = success && changeError == nil;
                                        completion(context, saved ? 1 : 0,
                                                   saved ? NULL : ((changeError ?: error.localizedDescription).UTF8String ?: "相册写入失败"));
                                    }
                                }];
                            } @catch (NSException *exception) {
                                completion(context, 0, exception.reason.UTF8String ?: "相册写入异常");
                            }
                        }
                    };
                    if (@available(iOS 14.0, *)) {
                        [PHPhotoLibrary requestAuthorizationForAccessLevel:PHAccessLevelAddOnly handler:authorized];
                    } else {
                        [PHPhotoLibrary requestAuthorization:authorized];
                    }
                } @catch (NSException *exception) {
                    completion(context, 0, exception.reason.UTF8String ?: "相册授权异常");
                }
            });
        } @catch (NSException *exception) {
            completion(context, 0, exception.reason.UTF8String ?: "图片导出异常");
        }
    }
}
