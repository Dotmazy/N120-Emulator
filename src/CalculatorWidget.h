#pragma once
#include <QWidget>
#include <QImage>
#include <QPixmap>
#include <QSet>
#include <QPainterPath>
#include <QVector3D>

struct CalculatorKey { int index; QString name; QRectF bounds; };
class CalculatorWidget final : public QWidget {
    Q_OBJECT
public:
    explicit CalculatorWidget(QWidget* parent=nullptr);
    static const QList<CalculatorKey>& keys();
    static QRectF screenBounds() { return {185,184,792,594}; }
    QRectF imageRect() const;
    QPointF toImage(QPointF p) const;
    int hitKey(QPointF p) const;
    void setFrame(const QByteArray& rgb565);
    QImage lcdImage() const { return frame; }
    void setHardware(QVector3D rgb,double backlight);
    void setHeld(const QSet<int>& held);
    void setHitboxes(bool show) { hitboxes=show;update(); }
    void setDim(bool value) {dim=value;update();}
    void setSmoothLcd(bool value) {smoothLcd=value;update();}
    void setZoom(double value) {zoom=value;updateGeometry();update();}
    QSize sizeHint() const override {return QSize(int(440*zoom),int(845*zoom));}
signals:
    void physicalKey(int index,bool pressed);
protected:
    void paintEvent(QPaintEvent*) override;
    void mouseMoveEvent(QMouseEvent*) override;
    void mousePressEvent(QMouseEvent*) override;
    void mouseReleaseEvent(QMouseEvent*) override;
    void leaveEvent(QEvent*) override;
private:
    QPixmap skin;
    QImage frame;
    QSet<int> held;
    int hover=-1,mouseKey=-1;
    bool hitboxes=false,dim=true,smoothLcd=false;
    double brightness=1,zoom=1;
    QVector3D rgb;
};
