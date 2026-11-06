#include "LcdItem.h"
#include <QPainter>

void LcdItem::setController(MobileController* value) {
    if (source)
        disconnect(source, nullptr, this, nullptr);
    source = value;
    if (source)
        connect(source, &MobileController::frameChanged, this, [this] { update(); });
    emit controllerChanged();
    update();
}

void LcdItem::paint(QPainter* painter) {
    painter->setRenderHint(QPainter::SmoothPixmapTransform, false);
    if (source)
        painter->drawImage(boundingRect(), source->frame());
    else
        painter->fillRect(boundingRect(), Qt::black);
}
