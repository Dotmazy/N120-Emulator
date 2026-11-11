#include "CalculatorWidget.h"
#include "resources/CalculatorStyle.h"
#include <QDir>
#include <QtTest>

class CalculatorAppearanceTests : public QObject {
    Q_OBJECT
  private slots:

    void stylesKeepTheFramebufferAndHitboxes() {
        CalculatorWidget widget;
        widget.resize(464, 889);
        widget.setFrame(QByteArray(320 * 240 * 2, char(0x7F)));
        const auto frame = widget.lcdImage();
        QCOMPARE(calculatorStyles().size(), 3);
        QImage previous;
        for (const auto& style : calculatorStyles()) {
            QVERIFY2(!QImage(style.resource).isNull(), qPrintable(style.resource));
            widget.setCalculatorStyle(style.id);
            QCOMPARE(widget.calculatorStyle(), style.id);
            QCOMPARE(widget.lcdImage(), frame);
            auto preview = widget.grab().toImage();
            if (!previous.isNull())
                QVERIFY(preview != previous);
            previous = preview;
            for (const auto& key : calculatorKeys())
                QCOMPARE(calculatorHitKey(key.bounds.center()), key.index);
            const auto output = qEnvironmentVariable("N120_STYLE_TEST_OUTPUT");
            if (!output.isEmpty()) {
                QDir().mkpath(output);
                QVERIFY(preview.save(output + '/' + style.id + ".png"));
            }
        }
        widget.setCalculatorStyle("unknown-style");
        QCOMPARE(widget.calculatorStyle(), QString("upsilon"));
    }

    void arrowsReachBeyondTheOldTinyEllipses() {
        // Outer shoulders, inner arms and diagonal shoulders, rather than only glyph centers.
        const QList<QPair<QPointF, int>> points = {
            {{126, 1030}, 0},  {{245, 1030}, 0}, {{277, 884}, 1},   {{277, 999}, 1}, {{277, 1176}, 2},
            {{277, 1061}, 2},  {{428, 1030}, 3}, {{309, 1030}, 3},  {{222, 940}, 1}, {{205, 985}, 0},
            {{349, 985}, 3},   {{222, 1120}, 2}, {{277, 1030}, -1}, {{0, 0}, -1},    {{580, 400}, -1},
            {{129, 1250}, 12}, {{199, 1576}, 30}};
        for (const auto& point : points)
            QCOMPARE(calculatorHitKey(point.first), point.second);
        CalculatorWidget widget;
        for (auto size : {QSize(290, 550), QSize(750, 850), QSize(500, 1100)}) {
            widget.resize(size);
            auto rect = widget.imageRect();
            for (const auto& point : points) {
                const QPointF screen(rect.x() + point.first.x() * rect.width() / 1160,
                                     rect.y() + point.first.y() * rect.height() / 2221);
                QCOMPARE(widget.hitKey(screen), point.second);
            }
        }
    }

    void targetsDoNotOverlap() {
        const auto& keys = calculatorKeys();
        for (int i = 0; i < keys.size(); ++i)
            for (int j = i + 1; j < keys.size(); ++j)
                QVERIFY2(calculatorKeyHitShape(keys[i]).intersected(calculatorKeyHitShape(keys[j])).isEmpty(),
                         qPrintable(keys[i].name + " / " + keys[j].name));
    }
};
QTEST_MAIN(CalculatorAppearanceTests)
#include "calculator_appearance_tests.moc"
