#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface NativeSVGPreflight : NSObject
+ (nullable NSData *)sanitizedData:(NSData *)data;
+ (nullable NSData *)sanitizedData:(NSData *)data
              resourceIdentifiers:(NSArray<NSString *> * _Nullable * _Nullable)resourceIdentifiers;
@end

NS_ASSUME_NONNULL_END
