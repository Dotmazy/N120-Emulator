#include "CalculatorLayout.h"
#include <QStringList>

const QList<CalculatorKey>& calculatorKeys() {
    // Bounds use the original 1160 x 2221 skin coordinates; painting and hit testing share this space.
    static const QList<CalculatorKey> result = [] {
        QList<CalculatorKey> k = {{0, "Left", {151, 989, 77, 78}},   {1, "Up", {238, 900, 78, 79}},
                                  {2, "Down", {238, 1078, 78, 80}},  {3, "Right", {325, 989, 78, 78}},
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
