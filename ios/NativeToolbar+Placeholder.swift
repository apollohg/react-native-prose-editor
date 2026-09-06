import UIKit

final class EditorAccessoryPlaceholderView: UIView {
    override init(frame: CGRect) {
        super.init(
            frame: CGRect(
                x: frame.origin.x,
                y: frame.origin.y,
                width: frame.width,
                height: 0
            )
        )
        commonInit()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: 0)
    }

    override func sizeThatFits(_ size: CGSize) -> CGSize {
        CGSize(width: size.width, height: 0)
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        false
    }

    private func commonInit() {
        frame.size.height = 0
        backgroundColor = .clear
        isOpaque = false
        isUserInteractionEnabled = false
        autoresizingMask = [.flexibleWidth]
    }
}
