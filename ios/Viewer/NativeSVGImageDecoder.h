#import <UIKit/UIKit.h>

NS_ASSUME_NONNULL_BEGIN

@interface NativeSVGImageDecoder : NSObject
+ (nullable UIImage *)decodeData:(NSData *)data maxDimension:(NSInteger)maxDimension;
@end

NS_ASSUME_NONNULL_END
