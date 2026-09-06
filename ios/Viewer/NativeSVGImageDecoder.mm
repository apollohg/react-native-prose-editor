#import "NativeSVGImageDecoder.h"
#import "NativeSVGPreflight.h"
#import <SVGgh/SVGRenderer.h>
#include <cmath>

static CGFloat SVGLength(NSString *value) {
    if (!value.length) return NAN;
    NSScanner *scanner = [NSScanner scannerWithString:value];
    scanner.locale = [NSLocale localeWithLocaleIdentifier:@"en_US_POSIX"];
    double number;
    if (![scanner scanDouble:&number] || !std::isfinite(number)) return NAN;
    NSString *unit = [[value substringFromIndex:scanner.scanLocation]
        stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet].lowercaseString;
    NSDictionary<NSString *, NSNumber *> *units = @{
        @"": @1, @"px": @1, @"in": @96, @"cm": @(96.0 / 2.54),
        @"mm": @(96.0 / 25.4), @"q": @(96.0 / 101.6), @"pt": @(96.0 / 72), @"pc": @16
    };
    NSNumber *multiplier = units[unit];
    return multiplier ? number * multiplier.doubleValue : NAN;
}

static BOOL SVGViewBox(NSString *value, CGRect *rect) {
    NSScanner *scanner = [NSScanner scannerWithString:value];
    scanner.locale = [NSLocale localeWithLocaleIdentifier:@"en_US_POSIX"];
    scanner.charactersToBeSkipped = [NSCharacterSet characterSetWithCharactersInString:@" ,\t\r\n"];
    double numbers[4];
    for (NSUInteger i = 0; i < 4; i++) {
        if (![scanner scanDouble:&numbers[i]] || !std::isfinite(numbers[i])) return NO;
    }
    if (!scanner.isAtEnd || numbers[2] <= 0 || numbers[3] <= 0) return NO;
    *rect = CGRectMake(numbers[0], numbers[1], numbers[2], numbers[3]);
    return YES;
}

@implementation NativeSVGImageDecoder

+ (UIImage *)decodeData:(NSData *)data maxDimension:(NSInteger)maxDimension {
    if (maxDimension <= 0 || maxDimension > 8192) return nil;
    NSArray<NSString *> *resourceIdentifiers;
    NSData *sanitized = [NativeSVGPreflight sanitizedData:data resourceIdentifiers:&resourceIdentifiers];
    if (!sanitized) return nil;
    @try {
        SVGRenderer *svg = [[SVGRenderer alloc] initWithInputStream:[NSInputStream inputStreamWithData:sanitized]];
        if (!svg || svg.parserError) return nil;
        for (NSString *identifier in resourceIdentifiers) {
            id resource = [svg objectNamed:identifier];
            if ([resource respondsToSelector:@selector(getClippingTypeWithSVGContext:)]) {
                ClippingType type = [(id<GHRenderable>)resource getClippingTypeWithSVGContext:svg];
                if (type == kMixedClippingType || type == kFontGlyphClippingType || type == kImageClipplingType) return nil;
            }
        }
        NSDictionary *attributes = svg.attributes;
        CGRect viewBox = CGRectZero;
        BOOL hasViewBox = [attributes[@"viewBox"] length] > 0;
        if (hasViewBox && !SVGViewBox(attributes[@"viewBox"], &viewBox)) return nil;
        CGFloat width = SVGLength(attributes[@"width"]);
        CGFloat height = SVGLength(attributes[@"height"]);
        if ((!std::isnan(width) && width <= 0) || (!std::isnan(height) && height <= 0)) return nil;
        if (std::isnan(width)) {
            width = hasViewBox ? (std::isnan(height) ? viewBox.size.width : height * viewBox.size.width / viewBox.size.height) : 300;
        }
        if (std::isnan(height)) {
            height = hasViewBox ? width * viewBox.size.height / viewBox.size.width : 150;
        }
        if (!std::isfinite(width) || !std::isfinite(height) || width <= 0 || height <= 0) return nil;
        if (!hasViewBox) viewBox = CGRectMake(0, 0, width, height);
        CGFloat ratio = MIN(1, maxDimension / MAX(width, height));
        CGSize size = CGSizeMake(
            MAX(1, MIN(maxDimension, std::floor(width * ratio))),
            MAX(1, MIN(maxDimension, std::floor(height * ratio)))
        );
        NSArray<NSString *> *parts = [attributes[@"preserveAspectRatio"] componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        NSMutableArray<NSString *> *tokens = [NSMutableArray array];
        for (NSString *part in parts) if (part.length && ![part isEqualToString:@"defer"]) [tokens addObject:part];
        NSString *alignment = tokens.firstObject ?: @"xMidYMid";
        BOOL stretch = [alignment isEqualToString:@"none"];
        BOOL slice = tokens.count > 1 && [tokens[1] isEqualToString:@"slice"];
        CGFloat scaleX = width / viewBox.size.width, scaleY = height / viewBox.size.height;
        if (!stretch) scaleX = scaleY = slice ? MAX(scaleX, scaleY) : MIN(scaleX, scaleY);
        CGFloat alignX = [alignment hasPrefix:@"xMin"] ? 0 : ([alignment hasPrefix:@"xMax"] ? 1 : 0.5);
        CGFloat alignY = [alignment hasSuffix:@"YMin"] ? 0 : ([alignment hasSuffix:@"YMax"] ? 1 : 0.5);
        CGFloat offsetX = stretch ? 0 : (width - viewBox.size.width * scaleX) * alignX;
        CGFloat offsetY = stretch ? 0 : (height - viewBox.size.height * scaleY) * alignY;
        CGAffineTransform transform = CGAffineTransformMake(
            scaleX * size.width / width, 0, 0, scaleY * size.height / height,
            (offsetX - viewBox.origin.x * scaleX) * size.width / width,
            (offsetY - viewBox.origin.y * scaleY) * size.height / height
        );
        if (!std::isfinite(transform.a) || !std::isfinite(transform.d) ||
            !std::isfinite(transform.tx) || !std::isfinite(transform.ty)) return nil;
        UIGraphicsImageRendererFormat *format = [[UIGraphicsImageRendererFormat alloc] init];
        format.scale = 1;
        format.opaque = NO;
        format.preferredRange = UIGraphicsImageRendererFormatRangeStandard;
        UIGraphicsImageRenderer *renderer = [[UIGraphicsImageRenderer alloc] initWithSize:size format:format];
        return [renderer imageWithActions:^(UIGraphicsImageRendererContext *context) {
            CGContextClipToRect(context.CGContext, CGRectMake(0, 0, size.width, size.height));
            CGContextConcatCTM(context.CGContext, transform);
            [svg renderIntoContext:context.CGContext];
        }];
    } @catch (NSException *exception) {
        return nil;
    }
}

@end
