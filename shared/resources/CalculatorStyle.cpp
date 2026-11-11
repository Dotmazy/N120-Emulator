#include "CalculatorStyle.h"

const QList<CalculatorStyle>& calculatorStyles() {
    static const QList<CalculatorStyle> styles = {{"epsilon", "Epsilon", ":/assets/epsilon-background.png"},
                                                  {"upsilon", "Upsilon", ":/assets/upsilon-background.png"},
                                                  {"omega", "Omega", ":/assets/omega-background.png"}};
    return styles;
}

QString normalizedCalculatorStyle(const QString& id) {
    for (const auto& style : calculatorStyles())
        if (style.id == id)
            return id;
    return "upsilon";
}

QString calculatorStyleResource(const QString& id) {
    const auto selected = normalizedCalculatorStyle(id);
    for (const auto& style : calculatorStyles())
        if (style.id == selected)
            return style.resource;
    return {};
}
