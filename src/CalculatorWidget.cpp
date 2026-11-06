#include "CalculatorWidget.h"
#include <QMouseEvent>
#include <QPainter>
#include <QtEndian>

const QList<CalculatorKey>& CalculatorWidget::keys() {
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

CalculatorWidget::CalculatorWidget(QWidget* parent)
    : QWidget(parent), skin(":/assets/calculator.png"), frame(320, 240, QImage::Format_RGB32) {
    setMouseTracking(true);
    setFocusPolicy(Qt::StrongFocus);
    setMinimumSize(290, 550);
    frame.fill(QColor("#202020"));
    setAccessibleName("NumWorks N120 calculator");
}

QRectF CalculatorWidget::imageRect() const {
    QSizeF s(1160, 2221);
    s.scale(size(), Qt::KeepAspectRatio);
    return {(width() - s.width()) / 2, (height() - s.height()) / 2, s.width(), s.height()};
}

QPointF CalculatorWidget::toImage(QPointF p) const {
    auto r = imageRect();
    return {(p.x() - r.x()) * 1160 / r.width(), (p.y() - r.y()) * 2221 / r.height()};
}

int CalculatorWidget::hitKey(QPointF p) const {
    auto point = toImage(p);
    for (const auto& k : keys()) {
        QPainterPath shape;
        shape.addEllipse(k.bounds);
        if (shape.contains(point))
            return k.index;
    }
    return -1;
}

void CalculatorWidget::setFrame(const QByteArray& bytes) {
    if (bytes.size() != 320 * 240 * 2)
        return;
    // The bridge sends little-endian RGB565; QImage owns the expanded pixels independently.
    for (int y = 0; y < 240; y++) {
        auto line = reinterpret_cast<QRgb*>(frame.scanLine(y));
        for (int x = 0; x < 320; x++) {
            auto p = qFromLittleEndian<quint16>(bytes.constData() + 2 * (y * 320 + x));
            line[x] = qRgb((p >> 11) * 255 / 31, ((p >> 5) & 63) * 255 / 63, (p & 31) * 255 / 31);
        }
    }
    auto r = imageRect();
    update(QRectF(r.x() + 185 * r.width() / 1160, r.y() + 184 * r.height() / 2221, 792 * r.width() / 1160,
                  594 * r.height() / 2221)
               .toAlignedRect());
}

void CalculatorWidget::setHardware(QVector3D value, double level) {
    if (rgb != value || brightness != level) {
        rgb = value;
        brightness = level;
        update();
    }
}

void CalculatorWidget::setHeld(const QSet<int>& value) {
    if (held != value) {
        held = value;
        update();
    }
}

void CalculatorWidget::paintEvent(QPaintEvent*) {
    QPainter p(this);
    auto r = imageRect();
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    p.drawPixmap(r, skin, skin.rect());
    p.save();
    p.translate(r.topLeft());
    p.scale(r.width() / 1160, r.height() / 2221);
    p.setRenderHint(QPainter::SmoothPixmapTransform, smoothLcd);
    p.drawImage(screenBounds(), frame);
    // Backlight dimming is a visual overlay; GRAM contents must survive power transitions.
    if (dim && brightness < 1)
        p.fillRect(screenBounds(), QColor(0, 0, 0, int(205 * (1 - brightness))));
    p.setRenderHint(QPainter::Antialiasing, true);
    for (const auto& k : keys()) {
        bool pressed = held.contains(k.index);
        if (pressed || hover == k.index || hitboxes) {
            p.setPen(hitboxes ? QPen(QColor("#38bdf8"), 3) : Qt::NoPen);
            p.setBrush(pressed            ? QColor(39, 130, 225, 110)
                       : hover == k.index ? QColor(80, 150, 225, 38)
                                          : QColor(38, 180, 200, 18));
            p.drawEllipse(k.bounds.adjusted(2, 2, -2, -2));
            if (hitboxes) {
                p.setPen(QColor("#155e75"));
                p.setFont(QFont("Segoe UI", 22, QFont::Bold));
                p.drawText(k.bounds, Qt::AlignCenter, QString::number(k.index));
            }
        }
    }
    QColor color = QColor::fromRgbF(rgb.x(), rgb.y(), rgb.z());
    double intensity = qMax(rgb.x(), qMax(rgb.y(), rgb.z()));
    if (intensity > 0) {
        QRadialGradient glow(QPointF(580, 28), 80);
        auto c = color;
        c.setAlpha(75);
        glow.setColorAt(0, c);
        glow.setColorAt(1, Qt::transparent);
        p.setPen(Qt::NoPen);
        p.setBrush(glow);
        p.drawEllipse(QRectF(490, -8, 180, 72));
    }
    QLinearGradient glass(0, 12, 0, 41);
    glass.setColorAt(0, intensity > 0 ? color.lighter(150) : QColor("#aab2bc"));
    glass.setColorAt(.55, intensity > 0 ? color : QColor("#303843"));
    glass.setColorAt(1, intensity > 0 ? color.darker(150) : QColor("#161d28"));
    p.setBrush(glass);
    p.setPen(QPen(QColor("#728093"), 2));
    p.drawEllipse(QRectF(535, 14, 90, 29));
    p.restore();
}

void CalculatorWidget::mouseMoveEvent(QMouseEvent* e) {
    int next = hitKey(e->position());
    if (next != hover) {
        hover = next;
        setCursor(hover >= 0 ? Qt::PointingHandCursor : Qt::ArrowCursor);
        for (const auto& k : keys())
            if (k.index == hover)
                setToolTip(k.name);
        if (hover < 0)
            setToolTip({});
        update();
    }
}

void CalculatorWidget::mousePressEvent(QMouseEvent* e) {
    if (e->button() != Qt::LeftButton)
        return;
    setFocus();
    mouseKey = hitKey(e->position());
    if (mouseKey >= 0)
        emit physicalKey(mouseKey, true);
}

void CalculatorWidget::mouseReleaseEvent(QMouseEvent* e) {
    if (e->button() == Qt::LeftButton && mouseKey >= 0) {
        emit physicalKey(mouseKey, false);
        mouseKey = -1;
    }
}

void CalculatorWidget::leaveEvent(QEvent*) {
    hover = -1;
    update();
}
