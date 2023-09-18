/*
 *     Copyright (C) 2023  Elliot Xu
 *
 *     This program is free software: you can redistribute it and/or modify
 *     it under the terms of the GNU Affero General Public License as published by
 *     the Free Software Foundation, either version 3 of the License, or
 *     (at your option) any later version.
 *
 *     This program is distributed in the hope that it will be useful,
 *     but WITHOUT ANY WARRANTY; without even the implied warranty of
 *     MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 *     GNU Affero General Public License for more details.
 *
 *     You should have received a copy of the GNU Affero General Public License
 *     along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

package com.elliot00.liushu.input.keyboard.candidate

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.elliot00.liushu.uniffi.Candidate
import com.elliot00.liushu.uniffi.CandidateSource

@Composable
fun CandidateItem(candidate: Candidate, onClick: () -> Unit = {}) {
    Box(
        modifier = Modifier
            .padding(horizontal = 12.dp)
            .clickable(onClick = onClick)
    ) {
        Text(
            text = candidate.text, textAlign = TextAlign.Center
        )
    }

}

@Preview
@Composable
private fun CandidateItemPreview() {
    CandidateItem(candidate = Candidate("hello", "hello", null, 1u, CandidateSource.CODE_TABLE))
}