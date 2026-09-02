package com.apollohg.editor;

import android.content.Context;
import android.util.AttributeSet;
import android.view.View;
import com.facebook.react.uimanager.PointerEvents;
import com.facebook.react.uimanager.ReactPointerEventsView;

public abstract class PointerTransparentView extends View implements ReactPointerEventsView {
    protected PointerTransparentView(Context context, AttributeSet attrs, int defStyleAttr) {
        super(context, attrs, defStyleAttr);
    }

    @Override
    public final PointerEvents getPointerEvents() {
        return PointerEvents.NONE;
    }
}
