#import "NativeSVGPreflight.h"

static const NSUInteger SVGMaxNodes = 8192;
static const NSUInteger SVGMaxDepth = 128;
static const NSUInteger SVGMaxBytes = 20 * 1024 * 1024;

@interface NativeSVGPreflight () <NSXMLParserDelegate>
@property(nonatomic) NSMutableData *output;
@property(nonatomic) NSMutableArray<NSMutableArray<NSNumber *> *> *edges;
@property(nonatomic) NSMutableArray<NSMutableArray<NSString *> *> *references;
@property(nonatomic) NSMutableDictionary<NSString *, NSNumber *> *identifiers;
@property(nonatomic) NSMutableOrderedSet<NSString *> *resourceIdentifiers;
@property(nonatomic) NSMutableArray<NSNumber *> *stack;
@property(nonatomic) NSUInteger byteLimit;
@property(nonatomic) NSUInteger referenceCount;
@property(nonatomic) NSMutableString *styleText;
@property(nonatomic) BOOL rejected;
@end

@implementation NativeSVGPreflight

+ (NSData *)sanitizedData:(NSData *)data {
    return [self sanitizedData:data resourceIdentifiers:nil];
}

+ (NSData *)sanitizedData:(NSData *)data resourceIdentifiers:(NSArray<NSString *> **)resourceIdentifiers {
    if (resourceIdentifiers) *resourceIdentifiers = nil;
    if (data.length == 0 || data.length > SVGMaxBytes) return nil;
    const uint8_t *bytes = (const uint8_t *)data.bytes;
    NSStringEncoding encoding = NSUTF8StringEncoding;
    if (data.length >= 4 && ((bytes[0] == 0 && bytes[1] == 0 && bytes[2] == 0xfe && bytes[3] == 0xff) ||
                            (bytes[0] == 0xff && bytes[1] == 0xfe && bytes[2] == 0 && bytes[3] == 0))) {
        encoding = NSUTF32StringEncoding;
    } else if (data.length >= 2 && ((bytes[0] == 0xff && bytes[1] == 0xfe) || (bytes[0] == 0xfe && bytes[1] == 0xff))) {
        encoding = NSUTF16StringEncoding;
    }
    NSString *source = [[NSString alloc] initWithData:data encoding:encoding];
    if (!source || [source rangeOfString:[NSString stringWithFormat:@"%C", (unichar)0]].location != NSNotFound || [source rangeOfString:@"<!DOCTYPE" options:NSCaseInsensitiveSearch].location != NSNotFound ||
        [source rangeOfString:@"<!ENTITY" options:NSCaseInsensitiveSearch].location != NSNotFound) return nil;

    NativeSVGPreflight *guard = [NativeSVGPreflight new];
    guard.output = [NSMutableData data];
    guard.edges = [NSMutableArray array];
    guard.references = [NSMutableArray array];
    guard.identifiers = [NSMutableDictionary dictionary];
    guard.resourceIdentifiers = [NSMutableOrderedSet orderedSet];
    guard.stack = [NSMutableArray array];
    guard.byteLimit = MIN(SVGMaxBytes, data.length * 8 + 1024);
    NSXMLParser *parser = [[NSXMLParser alloc] initWithData:data];
    parser.shouldResolveExternalEntities = NO;
    parser.shouldProcessNamespaces = NO;
    parser.delegate = guard;
    if (![parser parse] || guard.rejected || guard.edges.count == 0 || guard.stack.count != 0) return nil;
    for (NSUInteger i = 0; i < guard.references.count; i++) {
        for (NSString *identifier in guard.references[i]) {
            NSNumber *target = guard.identifiers[identifier];
            if (!target) return nil;
            [guard.edges[i] addObject:target];
        }
    }
    NSMutableData *states = [NSMutableData dataWithLength:guard.edges.count];
    NSMutableData *costs = [NSMutableData dataWithLength:guard.edges.count * sizeof(NSUInteger)];
    NSMutableData *depths = [NSMutableData dataWithLength:guard.edges.count * sizeof(NSUInteger)];
    if (![guard visit:0 states:(uint8_t *)states.mutableBytes costs:(NSUInteger *)costs.mutableBytes
                 depths:(NSUInteger *)depths.mutableBytes level:1]) return nil;
    if (resourceIdentifiers) *resourceIdentifiers = guard.resourceIdentifiers.array;
    return [guard.output copy];
}

- (BOOL)visit:(NSUInteger)node states:(uint8_t *)states costs:(NSUInteger *)costs
       depths:(NSUInteger *)depths level:(NSUInteger)level {
    if (level > SVGMaxDepth || states[node] == 1) return NO;
    if (states[node] == 2) return YES;
    states[node] = 1;
    NSUInteger cost = 1, depth = 1;
    for (NSNumber *edge in self.edges[node]) {
        NSUInteger child = edge.unsignedIntegerValue;
        if (![self visit:child states:states costs:costs depths:depths level:level + 1]) return NO;
        cost += costs[child];
        depth = MAX(depth, depths[child] + 1);
        if (cost > SVGMaxNodes || depth > SVGMaxDepth) return NO;
    }
    costs[node] = cost;
    depths[node] = depth;
    states[node] = 2;
    return YES;
}

- (void)reject:(NSXMLParser *)parser {
    self.rejected = YES;
    [parser abortParsing];
}

- (void)append:(NSString *)string parser:(NSXMLParser *)parser {
    NSData *data = [string dataUsingEncoding:NSUTF8StringEncoding];
    if (!data || data.length > self.byteLimit - self.output.length) {
        [self reject:parser];
        return;
    }
    [self.output appendData:data];
}

- (NSString *)escaped:(NSString *)value {
    return [[[[value stringByReplacingOccurrencesOfString:@"&" withString:@"&amp;"]
        stringByReplacingOccurrencesOfString:@"<" withString:@"&lt;"]
        stringByReplacingOccurrencesOfString:@">" withString:@"&gt;"]
        stringByReplacingOccurrencesOfString:@"\"" withString:@"&quot;"];
}

- (BOOL)checkValue:(NSString *)value node:(NSUInteger)node allowURLs:(BOOL)allowURLs {
    if ([value rangeOfString:@"\\"].location != NSNotFound || [value rangeOfString:@"/*"].location != NSNotFound ||
        [value rangeOfString:@"@"].location != NSNotFound) return NO;
    static NSRegularExpression *urls;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ urls = [NSRegularExpression regularExpressionWithPattern:@"url\\s*\\(([^)]*)\\)" options:NSRegularExpressionCaseInsensitive error:nil]; });
    __block BOOL valid = YES;
    [urls enumerateMatchesInString:value options:0 range:NSMakeRange(0, value.length) usingBlock:^(NSTextCheckingResult *match, NSMatchingFlags flags, BOOL *stop) {
        if (!allowURLs || ++self.referenceCount > SVGMaxNodes) { valid = NO; *stop = YES; return; }
        NSString *target = [[value substringWithRange:[match rangeAtIndex:1]] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        if (target.length >= 2 && (([target hasPrefix:@"'"] && [target hasSuffix:@"'"]) ||
                                  ([target hasPrefix:@"\""] && [target hasSuffix:@"\""]))) {
            target = [target substringWithRange:NSMakeRange(1, target.length - 2)];
        }
        if (![target hasPrefix:@"#"] || target.length < 2) { valid = NO; *stop = YES; return; }
        NSString *identifier = [target substringFromIndex:1];
        [self.references[node] addObject:identifier];
        [self.resourceIdentifiers addObject:identifier];
    }];
    if (!valid) return NO;
    NSString *remaining = [urls stringByReplacingMatchesInString:value options:0 range:NSMakeRange(0, value.length) withTemplate:@""];
    return [remaining rangeOfString:@"url\\s*\\(" options:NSRegularExpressionSearch | NSCaseInsensitiveSearch].location == NSNotFound;
}

- (void)parser:(NSXMLParser *)parser didStartElement:(NSString *)element namespaceURI:(NSString *)namespaceURI
 qualifiedName:(NSString *)qualifiedName attributes:(NSDictionary<NSString *, NSString *> *)attributes {
    static NSSet<NSString *> *allowed;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ allowed = [NSSet setWithArray:@[@"svg", @"g", @"defs", @"symbol", @"use", @"path", @"rect", @"circle", @"ellipse", @"line", @"polyline", @"polygon", @"text", @"tspan", @"textPath", @"linearGradient", @"radialGradient", @"stop", @"pattern", @"clipPath", @"mask", @"style", @"title", @"desc"]]; });
    NSString *local = [element componentsSeparatedByString:@":"].lastObject;
    if (self.styleText || ![allowed containsObject:local] || self.edges.count >= SVGMaxNodes || self.stack.count >= SVGMaxDepth ||
        (self.edges.count == 0 && ![local isEqualToString:@"svg"])) {
        [self reject:parser]; return;
    }
    NSUInteger node = self.edges.count;
    [self.edges addObject:[NSMutableArray array]];
    [self.references addObject:[NSMutableArray array]];
    if (self.stack.count) [self.edges[self.stack.lastObject.unsignedIntegerValue] addObject:@(node)];
    [self.stack addObject:@(node)];
    if ([local isEqualToString:@"style"]) self.styleText = [NSMutableString string];
    [self append:[@"<" stringByAppendingString:element] parser:parser];
    NSMutableDictionary<NSString *, NSString *> *normalized = [attributes mutableCopy];
    if (normalized[@"href"]) {
        normalized[@"xlink:href"] = normalized[@"href"];
        [normalized removeObjectForKey:@"href"];
    }
    if (node == 0 || normalized[@"xmlns:xlink"]) normalized[@"xmlns:xlink"] = @"http://www.w3.org/1999/xlink";
    for (NSString *name in normalized) {
        NSString *key = [name componentsSeparatedByString:@":"].lastObject.lowercaseString;
        NSString *value = normalized[name];
        BOOL declaration = [name isEqualToString:@"xmlns"] || [name hasPrefix:@"xmlns:"];
        BOOL literal = [@[@"id", @"href", @"title", @"class", @"aria-label"] containsObject:key] || [key hasPrefix:@"data-"];
        if (!declaration && ([key hasPrefix:@"on"] || [key isEqualToString:@"base"] ||
                             (!literal && ![self checkValue:value node:node allowURLs:YES]))) {
            [self reject:parser]; return;
        }
        if ([key isEqualToString:@"href"]) {
            if (++self.referenceCount > SVGMaxNodes || ![value hasPrefix:@"#"] || value.length < 2) { [self reject:parser]; return; }
            [self.references[node] addObject:[value substringFromIndex:1]];
        }
        if ([key isEqualToString:@"id"]) {
            if (value.length == 0 || self.identifiers[value]) { [self reject:parser]; return; }
            self.identifiers[value] = @(node);
        }
        [self append:[NSString stringWithFormat:@" %@=\"%@\"", name, [self escaped:value]] parser:parser];
        if (self.rejected) return;
    }
    [self append:@">" parser:parser];
}

- (void)parser:(NSXMLParser *)parser didEndElement:(NSString *)element namespaceURI:(NSString *)namespaceURI qualifiedName:(NSString *)qualifiedName {
    if (self.styleText) {
        if (![self checkValue:self.styleText node:self.stack.lastObject.unsignedIntegerValue allowURLs:NO]) {
            [self reject:parser]; return;
        }
        self.styleText = nil;
    }
    [self append:[NSString stringWithFormat:@"</%@>", element] parser:parser];
    [self.stack removeLastObject];
}

- (void)parser:(NSXMLParser *)parser foundCharacters:(NSString *)string {
    if (self.styleText) [self.styleText appendString:string];
    [self append:[self escaped:string] parser:parser];
}

- (void)parser:(NSXMLParser *)parser foundCDATA:(NSData *)CDATABlock {
    NSString *text = [[NSString alloc] initWithData:CDATABlock encoding:NSUTF8StringEncoding];
    if (!text) { [self reject:parser]; return; }
    [self parser:parser foundCharacters:text];
}

- (void)parser:(NSXMLParser *)parser foundProcessingInstructionWithTarget:(NSString *)target data:(NSString *)data {
    [self reject:parser];
}

- (void)parser:(NSXMLParser *)parser foundInternalEntityDeclarationWithName:(NSString *)name value:(NSString *)value {
    [self reject:parser];
}

- (void)parser:(NSXMLParser *)parser foundExternalEntityDeclarationWithName:(NSString *)name publicID:(NSString *)publicID systemID:(NSString *)systemID {
    [self reject:parser];
}

- (NSData *)parser:(NSXMLParser *)parser resolveExternalEntityName:(NSString *)name systemID:(NSString *)systemID {
    [self reject:parser];
    return nil;
}

@end
