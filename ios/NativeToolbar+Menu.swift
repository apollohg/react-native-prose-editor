import UIKit

@available(iOS 16.0, *)
final class ToolbarEditMenuPresenter: NSObject, UIEditMenuInteractionDelegate {
    private final class Presentation {
        weak var sourceButton: UIButton?
        let menuProvider: () -> UIMenu?

        init(sourceButton: UIButton, menuProvider: @escaping () -> UIMenu?) {
            self.sourceButton = sourceButton
            self.menuProvider = menuProvider
        }
    }

    lazy var interaction = UIEditMenuInteraction(delegate: self)

    private var presentations: [String: Presentation] = [:]
    private var activePresentationIdentifier: String?
    var presentationRequestCount = 0

    func toggle(from sourceButton: UIButton, menuProvider: @escaping () -> UIMenu?) {
        if activePresentation?.sourceButton === sourceButton {
            interaction.dismissMenu()
            return
        }
        guard let hostView = interaction.view else { return }
        let identifier = UUID().uuidString
        presentations[identifier] = Presentation(
            sourceButton: sourceButton,
            menuProvider: menuProvider
        )
        activePresentationIdentifier = identifier
        presentationRequestCount += 1
        let sourcePoint = sourceButton.convert(
            CGPoint(x: sourceButton.bounds.midX, y: sourceButton.bounds.midY),
            to: hostView
        )
        interaction.presentEditMenu(
            with: UIEditMenuConfiguration(identifier: identifier as NSString, sourcePoint: sourcePoint)
        )
    }

    func reloadVisibleMenu() {
        interaction.reloadVisibleMenu()
    }

    func dismiss() {
        interaction.dismissMenu()
        presentations.removeAll()
        activePresentationIdentifier = nil
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        menuFor configuration: UIEditMenuConfiguration,
        suggestedActions: [UIMenuElement]
    ) -> UIMenu? {
        presentation(for: configuration)?.menuProvider()
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        targetRectFor configuration: UIEditMenuConfiguration
    ) -> CGRect {
        guard let sourceButton = presentation(for: configuration)?.sourceButton,
              let hostView = interaction.view
        else {
            return .null
        }
        return sourceButton.convert(sourceButton.bounds, to: hostView)
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        willDismissMenuFor configuration: UIEditMenuConfiguration,
        animator: any UIEditMenuInteractionAnimating
    ) {
        guard let identifier = identifier(for: configuration) else { return }
        animator.addCompletion { [weak self] in
            guard let self else { return }
            self.presentations.removeValue(forKey: identifier)
            if self.activePresentationIdentifier == identifier {
                self.activePresentationIdentifier = nil
            }
        }
    }

    private var activePresentation: Presentation? {
        guard let activePresentationIdentifier else { return nil }
        return presentations[activePresentationIdentifier]
    }

    private func presentation(for configuration: UIEditMenuConfiguration) -> Presentation? {
        guard let identifier = identifier(for: configuration) else { return activePresentation }
        return presentations[identifier]
    }

    func identifier(for configuration: UIEditMenuConfiguration) -> String? {
        (configuration.identifier as? NSString).map(String.init)
    }
}
