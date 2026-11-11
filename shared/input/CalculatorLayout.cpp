#include "CalculatorLayout.h"
#include <QPolygonF>
#include <QStringList>

const QList<CalculatorKey>& calculatorKeys() {
    // Bounds use the original 1160 x 2221 skin coordinates; painting and hit testing share this space.
    static const QList<CalculatorKey> result = [] {
        QList<CalculatorKey> k = {{0, "Left", {133, 982, 112, 94}},  {1, "Up", {229, 890, 96, 112}},
                                  {2, "Down", {229, 1058, 96, 112}}, {3, "Right", {309, 982, 112, 94}},
                                  {6, "Home", {503, 907, 158, 103}}, {8, "Power", {503, 1043, 158, 105}},
                                  {4, "OK", {751, 967, 122, 122}},   {5, "Back", {903, 967, 122, 122}}};
        QStringList names = {"Shift", "Alpha", "x,n,t", "Variable", "Toolbox", "Backspace",
                             "exp",   "ln",    "log",   "i",        "Comma",   "Power x^y",
                             "sin",   "cos",   "tan",   "Pi",       "Root",    "Square"};
        for (int r = 0; r < 3; r++)
            for (int c = 0; c < 6; c++)
                k.append(
                    {12 + r * 6 + c, names[r * 6 + c], {135.0 + 153.0 * c, 1213.0 + 122.0 * r, 126, 86}});
        QStringList numbers = {"7",     "8",        "9",       "(",        ")",      "4",  "5",
                               "6",     "Multiply", "Divide",  "1",        "2",      "3",  "Plus",
                               "Minus", "0",        "Decimal", "Exponent", "Answer", "EXE"};
        // Numeric rows have five visible keys but retain the hardware matrix stride of six.
        for (int r = 0; r < 4; r++)
            for (int c = 0; c < 5; c++)
                k.append(
                    {30 + r * 6 + c, numbers[r * 5 + c], {136.0 + 184.5 * c, 1584.0 + 137.0 * r, 151, 102}});
        return k;
    }();
    return result;
}

QPainterPath calculatorKeyHitShape(const CalculatorKey& key) {
    QPainterPath shape;
    if (key.index >= 0 && key.index <= 3) {
        // Four non-overlapping sectors cover the cross and nearby gaps. The center stays neutral.
        const QPointF center(277, 1030);
        const QPointF tl(119, 872), tr(435, 872), bl(119, 1188), br(435, 1188);
        QPolygonF sector;
        switch (key.index) {
        case 0:
            sector = {center, bl, tl};
            break;
        case 1:
            sector = {center, tl, tr};
            break;
        case 2:
            sector = {center, br, bl};
            break;
        case 3:
            sector = {center, tr, br};
            break;
        }
        shape.addPolygon(sector);
        shape.closeSubpath();
        QPainterPath outer, neutral;
        outer.addRoundedRect(QRectF(tl, br), 24, 24);
        neutral.addEllipse(center, 18, 18);
        return shape.intersected(outer).subtracted(neutral);
    }
    // Padding stays within the gaps between adjacent keys, including the tightly spaced function rows.
    shape.addRoundedRect(key.bounds.adjusted(-9, -12, 9, 12), 22, 22);
    return shape;
}

int calculatorHitKey(QPointF position) {
    // Cache the paths: desktop hover and Android multi-touch use exactly the same targets.
    static const auto shapes = [] {
        QList<QPair<int, QPainterPath>> result;
        for (const auto& key : calculatorKeys())
            result.append({key.index, calculatorKeyHitShape(key)});
        return result;
    }();
    for (const auto& shape : shapes)
        if (shape.second.contains(position))
            return shape.first;
    return -1;
}
